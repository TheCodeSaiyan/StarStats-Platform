//! Transactional email transport.
//!
//! Two implementations of the [`Mailer`] trait:
//!
//!  * [`LettreMailer`] — async SMTP via `lettre`. Built once at startup
//!    from [`SmtpConfig`] and shared via `Arc<dyn Mailer>` so handlers
//!    don't reconnect per request. The transport reuses TLS connections
//!    internally; we treat `send` as fire-and-forget from the caller's
//!    perspective.
//!
//!  * [`NoopMailer`] — used when `SMTP_URL` isn't configured. Logs the
//!    intended send and returns `Ok(())`, which lets local dev (and the
//!    test suite) exercise the full signup → verify path without
//!    standing up a real SMTP server.
//!
//! The verification link format is `${web_origin}/auth/verify?token=…`.
//! That URL is rendered by the Next.js `app/auth/verify/page.tsx`
//! server component, which calls back into `POST /v1/auth/email/verify`
//! with the token from the query string.
//!
//! Errors are surfaced as `anyhow::Error` because the caller (signup)
//! treats every failure here as best-effort — a warn-and-continue path
//! rather than a 500 to the user.

use anyhow::{Context, Result};
use async_trait::async_trait;
use lettre::address::Address;
use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::sync::Arc;

use crate::config::SmtpConfig;
use crate::smtp_config_store::SmtpConfigRecord;

/// Pluggable mailer interface so handlers can be parameterised over
/// "send" without dragging in a real SMTP transport in tests.
#[async_trait]
pub trait Mailer: Send + Sync + 'static {
    /// Send the verification email for `to_addr`. `to_name` is the
    /// recipient's display name (we use the claimed RSI handle —
    /// it's the most user-recognisable identifier we have at signup).
    async fn send_verification(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()>;

    /// Send the password-reset email. Same shape as verification —
    /// the link lives on the web app and posts the token back to the
    /// reset-complete endpoint.
    async fn send_password_reset(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()>;

    /// Send a "confirm your new email address" link to the *new*
    /// address while the old address remains the login until the
    /// link is clicked. `to_name` is the user's claimed handle.
    async fn send_email_change_verify(
        &self,
        to_addr: &str,
        to_name: &str,
        token: &str,
    ) -> Result<()>;

    /// Send a one-shot magic-link sign-in. Same shape as the
    /// password-reset flow but the redeemer gets a session JWT
    /// directly instead of bumping a hash.
    async fn send_magic_link(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()>;

    /// Send a diagnostic "is this thing on?" email. Used by the admin
    /// SMTP config page after a config save — there's no token because
    /// there's no flow to redeem; just a short body confirming the
    /// transport works end-to-end. `NoopMailer` logs without sending.
    async fn send_test_email(&self, to_addr: &str, to_name: &str) -> Result<()>;

    /// Send a beta invite to someone the waitlist just admitted.
    ///
    /// Note the missing `to_name`, unlike every method above: a waitlist
    /// signup is an email address and nothing else. There is no account,
    /// so no claimed handle to greet them by — and inventing one would be
    /// worse than not greeting them at all.
    async fn send_waitlist_invite(&self, to_addr: &str, invite_token: &str) -> Result<()>;
}

// -- Lettre (real SMTP) ----------------------------------------------

/// Async SMTP mailer wrapping `lettre::AsyncSmtpTransport`.
///
/// Construction parses `SMTP_URL` once and persists the resulting
/// transport. Lettre internally handles connection pooling and TLS;
/// the caller just hands it a `Message`.
pub struct LettreMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from_addr: String,
    from_name: String,
    web_origin: String,
}

impl LettreMailer {
    /// Build a `LettreMailer` from resolved config. Returns an error
    /// only when the URL fails to parse — there's no network probe at
    /// construction time, since Lettre opens connections lazily on
    /// first send.
    pub fn from_config(cfg: &SmtpConfig) -> Result<Self> {
        let transport =
            build_transport(&cfg.url).with_context(|| format!("parse SMTP_URL `{}`", cfg.url))?;
        Ok(Self {
            transport,
            from_addr: cfg.from_addr.clone(),
            from_name: cfg.from_name.clone(),
            web_origin: cfg.web_origin.trim_end_matches('/').to_string(),
        })
    }

    /// Build a `LettreMailer` from a DB-stored [`SmtpConfigRecord`].
    /// Skips the URL parser entirely — record fields map straight onto
    /// the Lettre builder. Returns an error if the host is blank
    /// (defensive: the admin form should refuse to save such a record
    /// but we surface a clean error here too).
    pub fn from_record(rec: &SmtpConfigRecord) -> Result<Self> {
        if rec.host.trim().is_empty() {
            anyhow::bail!("smtp record has empty host");
        }
        let transport = build_transport_from_parts(
            &rec.host,
            rec.port,
            &rec.username,
            rec.password.as_deref(),
            rec.secure,
        )
        .with_context(|| format!("build smtp transport for host `{}`", rec.host))?;
        Ok(Self {
            transport,
            from_addr: rec.from_addr.clone(),
            from_name: rec.from_name.clone(),
            web_origin: rec.web_origin.trim_end_matches('/').to_string(),
        })
    }
}

/// Parsed pieces of an SMTP URL. Avoids a `url` crate dependency by
/// hand-rolling the small, well-defined subset we accept:
/// `smtp[s]://[user[:pass]@]host[:port]`.
struct ParsedSmtpUrl {
    secure: bool,
    host: String,
    port: u16,
    username: String,
    password: String,
}

fn parse_smtp_url(url: &str) -> Result<ParsedSmtpUrl> {
    let (scheme, rest) = url
        .split_once("://")
        .with_context(|| format!("SMTP_URL missing scheme: `{url}`"))?;
    let secure = match scheme {
        "smtps" => true,
        "smtp" => false,
        other => anyhow::bail!("SMTP_URL has unsupported scheme `{other}`"),
    };

    // Optional userinfo before the last '@'. Use rsplit so passwords
    // containing '@' (rare but legal) don't break parsing.
    let (userinfo, host_and_port) = match rest.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, rest),
    };

    let (host, port) = match host_and_port.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p
                .parse()
                .with_context(|| format!("SMTP_URL port `{p}` is not a number"))?;
            (h.to_string(), port)
        }
        None => (host_and_port.to_string(), if secure { 465 } else { 587 }),
    };
    if host.is_empty() {
        anyhow::bail!("SMTP_URL missing host: `{url}`");
    }

    let (username, password) = match userinfo {
        None => (String::new(), String::new()),
        Some(ui) => match ui.split_once(':') {
            Some((u, p)) => (u.to_string(), p.to_string()),
            None => (ui.to_string(), String::new()),
        },
    };

    Ok(ParsedSmtpUrl {
        secure,
        host,
        port,
        username,
        password,
    })
}

/// Build a transport from the URL. `smtps://` -> implicit TLS (465);
/// `smtp://` -> STARTTLS (587). Lettre dials lazily on the first send.
fn build_transport(url: &str) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let parsed = parse_smtp_url(url)?;
    let mut builder = if parsed.secure {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&parsed.host)?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&parsed.host)?
    };
    builder = builder.port(parsed.port);
    if !parsed.username.is_empty() {
        builder = builder.credentials(Credentials::new(parsed.username, parsed.password));
    }
    Ok(builder.build())
}

/// Build a transport from split fields (the DB-stored shape). Same
/// semantics as `build_transport` but bypasses URL parsing — passwords
/// with `@`, `:` or `%` characters round-trip without any encoding
/// dance because we never serialise them into a URL.
fn build_transport_from_parts(
    host: &str,
    port: i32,
    username: &str,
    password: Option<&str>,
    secure: bool,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let port_u16: u16 =
        u16::try_from(port).with_context(|| format!("smtp port {port} out of range for u16"))?;
    let mut builder = if secure {
        AsyncSmtpTransport::<Tokio1Executor>::relay(host)?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?
    };
    builder = builder.port(port_u16);
    if !username.is_empty() {
        let pw = password.unwrap_or("").to_string();
        builder = builder.credentials(Credentials::new(username.to_string(), pw));
    }
    Ok(builder.build())
}

#[async_trait]
impl Mailer for LettreMailer {
    async fn send_verification(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        self.send(
            to_addr,
            to_name,
            "Verify your StarStats email",
            render_verification_body(&self.web_origin, token),
        )
        .await
    }

    async fn send_password_reset(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        self.send(
            to_addr,
            to_name,
            "Reset your StarStats password",
            render_password_reset_body(&self.web_origin, token),
        )
        .await
    }

    async fn send_email_change_verify(
        &self,
        to_addr: &str,
        to_name: &str,
        token: &str,
    ) -> Result<()> {
        self.send(
            to_addr,
            to_name,
            "Confirm your new StarStats email",
            render_email_change_body(&self.web_origin, token),
        )
        .await
    }

    async fn send_magic_link(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        self.send(
            to_addr,
            to_name,
            "Sign in to StarStats",
            render_magic_link_body(&self.web_origin, token),
        )
        .await
    }

    async fn send_test_email(&self, to_addr: &str, to_name: &str) -> Result<()> {
        self.send(
            to_addr,
            to_name,
            "StarStats — SMTP test",
            render_test_body(&self.from_addr, &self.web_origin),
        )
        .await
    }

    async fn send_waitlist_invite(&self, to_addr: &str, invite_token: &str) -> Result<()> {
        // to_addr doubles as the display name: a waitlist signup has no
        // account and therefore no handle to greet them by.
        self.send(
            to_addr,
            to_addr,
            "You're in — StarStats beta",
            render_waitlist_invite_body(&self.web_origin, invite_token),
        )
        .await
    }
}

/// Build a [`Mailbox`] from an address plus an optional display name.
///
/// Deliberately does NOT go via `format!("{name} <{addr}>")` + `parse()`.
/// A display name is an RFC 5322 `phrase` — a sequence of atoms, and
/// `atext` excludes `@`, `.`, `,`, `<`, `>` and friends — so formatting an
/// arbitrary name into a header string produces something that cannot be
/// parsed back. That is exactly how every waitlist invite broke: the
/// waitlist path uses the recipient's email AS the display name, giving
/// `a@b.com <a@b.com>`, which fails to parse.
///
/// Constructing the `Mailbox` directly removes the round-trip entirely and
/// lets lettre quote/encode the name when it renders the header. A display
/// name equal to the address is dropped — it carries no information, and
/// clients show the address anyway.
fn mailbox(addr: &str, name: &str) -> Result<Mailbox> {
    let address: Address = addr
        .trim()
        .parse()
        .with_context(|| format!("invalid email address: {addr}"))?;
    let name = name.trim();
    let display = (!name.is_empty() && name != addr.trim()).then(|| name.to_string());
    Ok(Mailbox::new(display, address))
}

impl LettreMailer {
    /// Shared envelope construction so the three send_* paths don't
    /// re-implement From/To header parsing or the SMTP send call.
    async fn send(&self, to_addr: &str, to_name: &str, subject: &str, body: String) -> Result<()> {
        let from = mailbox(&self.from_addr, &self.from_name).context("parse From address")?;
        let to = mailbox(to_addr, to_name).context("parse To address")?;
        let msg = Message::builder()
            .from(from)
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .context("build message")?;
        self.transport.send(msg).await.context("SMTP send failed")?;
        Ok(())
    }
}

/// Render the plain-text email body. Single template — short, no
/// HTML — because the recipient is a single human and we want this to
/// render identically in every client.
fn render_verification_body(web_origin: &str, token: &str) -> String {
    let link = format!("{web_origin}/auth/verify?token={token}");
    format!(
        "Welcome to StarStats!\n\
         \n\
         Click the link below to confirm your email address:\n\
         \n\
         {link}\n\
         \n\
         This link expires in 24 hours. If you didn't sign up, you can\n\
         ignore this message.\n"
    )
}

fn render_password_reset_body(web_origin: &str, token: &str) -> String {
    let link = format!("{web_origin}/auth/reset-password?token={token}");
    format!(
        "Someone (hopefully you) requested a password reset for your\n\
         StarStats account. Click the link below to choose a new\n\
         password:\n\
         \n\
         {link}\n\
         \n\
         This link expires in 30 minutes. If you didn't request a\n\
         reset, you can ignore this message — your password stays\n\
         unchanged.\n\
         \n\
         For your security, all paired devices and active sessions\n\
         will be signed out as soon as the password is changed.\n"
    )
}

fn render_email_change_body(web_origin: &str, token: &str) -> String {
    let link = format!("{web_origin}/auth/email-change?token={token}");
    format!(
        "You asked to change the email address on your StarStats\n\
         account to this one. Click the link below to confirm:\n\
         \n\
         {link}\n\
         \n\
         This link expires in 24 hours. Your old email continues to\n\
         work as your login until you click the link, so a typo here\n\
         won't lock you out — you can simply ignore this message.\n"
    )
}

fn render_magic_link_body(web_origin: &str, token: &str) -> String {
    let link = format!("{web_origin}/auth/magic-link/redeem?token={token}");
    format!(
        "Someone (hopefully you) asked to sign in to StarStats using\n\
         a magic link. Click below to finish signing in:\n\
         \n\
         {link}\n\
         \n\
         This link expires in 15 minutes and can only be used once.\n\
         If you didn't request a sign-in link, you can ignore this\n\
         message — no action will be taken on your account.\n"
    )
}

fn render_waitlist_invite_body(web_origin: &str, invite_token: &str) -> String {
    let link = format!("{web_origin}/auth/signup?invite={invite_token}");
    format!(
        "You asked to join the StarStats beta — you're in.\n\
         \n\
         Create your account here:\n\
         \n\
         {link}\n\
         \n\
         This link is for you alone and works once.\n\
         \n\
         StarStats is a beta run by one person. Things will break, and\n\
         that's largely why you're here — if something looks wrong, please\n\
         say so.\n\
         \n\
         If you didn't ask for this, ignore this message. No account\n\
         exists until that link is used.\n"
    )
}

fn render_test_body(from_addr: &str, web_origin: &str) -> String {
    format!(
        "This is a diagnostic email from the StarStats admin SMTP\n\
         config page. If you're reading this, the new SMTP settings\n\
         can successfully deliver mail end-to-end.\n\
         \n\
         From: {from_addr}\n\
         Origin: {web_origin}\n\
         \n\
         No action required.\n"
    )
}

// -- Noop (no SMTP configured) ---------------------------------------

/// Fallback mailer used when `SMTP_URL` is missing.
///
/// Logs the would-be send at info level — useful in dev where the
/// console doubles as the inbox — and otherwise returns Ok. Never
/// fails, so signup paths can call it unconditionally.
pub struct NoopMailer;

#[async_trait]
impl Mailer for NoopMailer {
    async fn send_verification(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        tracing::info!(
            to = to_addr,
            name = to_name,
            token,
            "noop mailer: would send verification email"
        );
        Ok(())
    }

    async fn send_password_reset(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        tracing::info!(
            to = to_addr,
            name = to_name,
            token,
            "noop mailer: would send password reset email"
        );
        Ok(())
    }

    async fn send_email_change_verify(
        &self,
        to_addr: &str,
        to_name: &str,
        token: &str,
    ) -> Result<()> {
        tracing::info!(
            to = to_addr,
            name = to_name,
            token,
            "noop mailer: would send email-change verification"
        );
        Ok(())
    }

    async fn send_magic_link(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        tracing::info!(
            to = to_addr,
            name = to_name,
            token,
            "noop mailer: would send magic-link"
        );
        Ok(())
    }

    async fn send_test_email(&self, to_addr: &str, to_name: &str) -> Result<()> {
        tracing::info!(
            to = to_addr,
            name = to_name,
            "noop mailer: would send test email"
        );
        Ok(())
    }

    async fn send_waitlist_invite(&self, to_addr: &str, invite_token: &str) -> Result<()> {
        tracing::info!(
            to = to_addr,
            token = invite_token,
            "noop mailer: would send waitlist invite"
        );
        Ok(())
    }
}

/// Build an `Arc<dyn Mailer>` from a DB-stored record. Always
/// succeeds — a malformed record (empty host, bad port) falls back to
/// `NoopMailer` with a warning, matching the env-driven `build_mailer`
/// posture. Caller is responsible for only calling this when the
/// record's `enabled` flag is true.
pub fn build_mailer_from_record(rec: &SmtpConfigRecord) -> Arc<dyn Mailer> {
    match LettreMailer::from_record(rec) {
        Ok(m) => {
            tracing::info!(
                from = %rec.from_addr,
                host = %rec.host,
                "SMTP mailer initialised from DB record"
            );
            Arc::new(m)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "DB SMTP record invalid; falling back to noop mailer"
            );
            Arc::new(NoopMailer)
        }
    }
}

// -- Swappable wrapper -----------------------------------------------

/// `Mailer` impl that wraps an inner `Arc<dyn Mailer>` behind a
/// `std::sync::RwLock`, allowing the admin save flow to replace the
/// active transport at runtime without restarting the server.
///
/// The read-side critical section is microscopic — clone an `Arc` and
/// drop the lock — so contention is irrelevant compared to the network
/// IO of an actual SMTP send.
///
/// Callers continue to hold `Arc<dyn Mailer>` (because `SwappableMailer`
/// implements `Mailer`); the admin route holds an `Arc<SwappableMailer>`
/// in addition so it can call [`Self::swap`].
pub struct SwappableMailer {
    inner: std::sync::RwLock<Arc<dyn Mailer>>,
}

impl SwappableMailer {
    pub fn new(initial: Arc<dyn Mailer>) -> Self {
        Self {
            inner: std::sync::RwLock::new(initial),
        }
    }

    /// Replace the active transport. Old `Arc`s held by in-flight
    /// sends keep working until they drop — Lettre's transport is
    /// itself an `Arc` internally, so the old transport stays alive
    /// for any send already past the read-lock acquisition.
    pub fn swap(&self, new: Arc<dyn Mailer>) {
        let mut guard = self.inner.write().expect("swappable mailer poisoned");
        *guard = new;
    }

    fn current(&self) -> Arc<dyn Mailer> {
        self.inner
            .read()
            .expect("swappable mailer poisoned")
            .clone()
    }
}

#[async_trait]
impl Mailer for SwappableMailer {
    async fn send_verification(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        self.current()
            .send_verification(to_addr, to_name, token)
            .await
    }

    async fn send_password_reset(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        self.current()
            .send_password_reset(to_addr, to_name, token)
            .await
    }

    async fn send_email_change_verify(
        &self,
        to_addr: &str,
        to_name: &str,
        token: &str,
    ) -> Result<()> {
        self.current()
            .send_email_change_verify(to_addr, to_name, token)
            .await
    }

    async fn send_magic_link(&self, to_addr: &str, to_name: &str, token: &str) -> Result<()> {
        self.current()
            .send_magic_link(to_addr, to_name, token)
            .await
    }

    async fn send_test_email(&self, to_addr: &str, to_name: &str) -> Result<()> {
        self.current().send_test_email(to_addr, to_name).await
    }

    async fn send_waitlist_invite(&self, to_addr: &str, invite_token: &str) -> Result<()> {
        self.current()
            .send_waitlist_invite(to_addr, invite_token)
            .await
    }
}

/// Build the runtime mailer based on config. Always succeeds — a
/// malformed `SMTP_URL` falls back to Noop with a warning, matching
/// the SpiceDB / MinIO degraded-boot posture.
pub fn build_mailer(cfg: Option<&SmtpConfig>) -> Arc<dyn Mailer> {
    match cfg {
        Some(c) => match LettreMailer::from_config(c) {
            Ok(m) => {
                tracing::info!(from = %c.from_addr, "SMTP mailer initialised");
                Arc::new(m)
            }
            Err(e) => {
                tracing::warn!(error = %e, "SMTP init failed; falling back to noop mailer");
                Arc::new(NoopMailer)
            }
        },
        None => {
            tracing::info!("SMTP not configured; using noop mailer (no verification emails)");
            Arc::new(NoopMailer)
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// A `Mailer` that records instead of sending, so handler tests can
    /// assert an email was actually attempted.
    ///
    /// This matters more than it looks: every send path here is
    /// best-effort by design (a mail failure must never 500 a request
    /// that already committed), which means a handler that silently
    /// forgets to send at all looks identical to a healthy one. These
    /// recordings are the only thing that can tell the difference.
    #[derive(Default)]
    pub struct RecordingMailer {
        waitlist: Mutex<Vec<(String, String)>>,
    }

    impl RecordingMailer {
        /// `(to_addr, invite_token)` for every waitlist invite attempted.
        pub fn waitlist_invites(&self) -> Vec<(String, String)> {
            self.waitlist.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Mailer for RecordingMailer {
        async fn send_verification(&self, _a: &str, _n: &str, _t: &str) -> Result<()> {
            Ok(())
        }
        async fn send_password_reset(&self, _a: &str, _n: &str, _t: &str) -> Result<()> {
            Ok(())
        }
        async fn send_email_change_verify(&self, _a: &str, _n: &str, _t: &str) -> Result<()> {
            Ok(())
        }
        async fn send_magic_link(&self, _a: &str, _n: &str, _t: &str) -> Result<()> {
            Ok(())
        }
        async fn send_test_email(&self, _a: &str, _n: &str) -> Result<()> {
            Ok(())
        }
        async fn send_waitlist_invite(&self, to_addr: &str, invite_token: &str) -> Result<()> {
            self.waitlist
                .lock()
                .unwrap()
                .push((to_addr.to_string(), invite_token.to_string()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Why the mailer log sites format with `{e:#}` and not `%e`.
    ///
    /// `Display` on an `anyhow::Error` prints ONLY the outermost context.
    /// Every mail failure is wrapped (`.context("parse To address")`,
    /// `.context("SMTP send failed")`, …), so `error = %e` logs the wrapper
    /// and silently discards the actual cause. That is exactly how the
    /// waitlist invite outage read as "the mail transport is broken" for as
    /// long as it did — the logs only ever said `parse To address`, never
    /// which address or why.
    ///
    /// The alternate form `{e:#}` flattens the whole chain onto one line,
    /// which keeps it greppable in the JSON log pipeline (unlike `?e`,
    /// whose `Debug` output is multi-line).
    #[test]
    fn alternate_display_surfaces_the_cause_chain_that_plain_display_hides() {
        let err = anyhow::anyhow!("invalid email address: a@b.com <a@b.com>")
            .context("parse To address")
            .context("send_waitlist_invite failed");

        let plain = format!("{err}");
        let chained = format!("{err:#}");

        // What `%e` would have logged: the wrapper only.
        assert_eq!(plain, "send_waitlist_invite failed");
        assert!(
            !plain.contains("invalid email address"),
            "plain Display hides the root cause — this is the bug being guarded against"
        );

        // What `{e:#}` logs: every layer, root cause included.
        assert!(chained.contains("send_waitlist_invite failed"));
        assert!(chained.contains("parse To address"));
        assert!(
            chained.contains("invalid email address: a@b.com <a@b.com>"),
            "the root cause must survive into the log line, got {chained}"
        );
        assert!(
            !chained.contains('\n'),
            "must stay single-line for the JSON log pipeline, got {chained}"
        );
    }

    /// Repro for the production failure: every waitlist invite/resend
    /// died with `parse To address`.
    ///
    /// `send_waitlist_invite` passes the email address as BOTH address
    /// and display name ("a waitlist signup has no handle to greet them
    /// by"), and `send` built the header as `format!("{name} <{addr}>")`.
    /// That yields `a@b.com <a@b.com>` — invalid per RFC 5322, because an
    /// unquoted display name is a `phrase` of atoms and `@` is not `atext`.
    /// So the round-trip through a formatted string could never parse.
    #[test]
    fn email_address_is_not_a_valid_unquoted_display_name() {
        let addr = "ntatschner@gmail.com";
        let legacy = format!("{addr} <{addr}>");
        assert!(
            legacy.parse::<Mailbox>().is_err(),
            "the old format! round-trip is what broke the transport"
        );
    }

    /// The fix: construct the `Mailbox` directly instead of formatting a
    /// header and re-parsing it. Covers the cases the old path mangled.
    #[test]
    fn mailbox_builds_without_round_tripping_through_a_header_string() {
        // Email-as-display-name (the waitlist case): the redundant display
        // name is dropped rather than emitted unquoted.
        let m = mailbox("ntatschner@gmail.com", "ntatschner@gmail.com").expect("email-as-name");
        assert_eq!(m.name, None, "a display name equal to the address is noise");
        assert_eq!(m.to_string(), "ntatschner@gmail.com");

        // Plus-addressing must survive untouched.
        let m = mailbox("ntatschner+beta1@gmail.com", "ntatschner+beta1@gmail.com")
            .expect("plus-addressing");
        assert_eq!(m.to_string(), "ntatschner+beta1@gmail.com");

        // A real display name is kept.
        let m = mailbox("a@example.com", "Alice").expect("plain name");
        assert_eq!(m.name.as_deref(), Some("Alice"));

        // A name containing RFC-special characters must not produce an
        // unparsable header — this is the class of bug, not just `@`.
        let m = mailbox("a@example.com", "J. Smith").expect("dotted name");
        assert!(
            m.to_string().parse::<Mailbox>().is_ok(),
            "rendered mailbox must round-trip, got {m}"
        );

        // A genuinely invalid address still errors.
        assert!(mailbox("not-an-address", "x").is_err());
    }

    #[tokio::test]
    async fn noop_mailer_returns_ok() {
        let m = NoopMailer;
        assert!(m
            .send_verification("a@example.com", "Alice", "tok")
            .await
            .is_ok());
    }

    #[test]
    fn waitlist_invite_body_links_signup_with_the_token() {
        let body = render_waitlist_invite_body("https://starstats.app", "deadbeef");
        // The link IS the email. Without it the invite is undeliverable
        // in the only sense that matters.
        assert!(body.contains("https://starstats.app/auth/signup?invite=deadbeef"));
        assert!(body.contains("works once"));
    }

    #[test]
    fn waitlist_invite_body_honours_the_configured_origin() {
        let body = render_waitlist_invite_body("http://localhost:3000", "tok123");
        assert!(body.contains("http://localhost:3000/auth/signup?invite=tok123"));
        // A hardcoded production origin would send every local tester to
        // the live site to redeem a token the live site has never heard of.
        assert!(!body.contains("starstats.app"));
    }

    #[tokio::test]
    async fn recording_mailer_captures_waitlist_invites() {
        let m = test_support::RecordingMailer::default();
        m.send_waitlist_invite("a@example.com", "tok")
            .await
            .unwrap();
        assert_eq!(
            m.waitlist_invites(),
            vec![("a@example.com".to_string(), "tok".to_string())]
        );
    }

    #[test]
    fn render_verification_body_includes_link_and_token() {
        let body = render_verification_body("https://app.example.com", "abc123");
        assert!(body.contains("https://app.example.com/auth/verify?token=abc123"));
        assert!(body.contains("expires in 24 hours"));
    }

    #[test]
    fn render_password_reset_body_includes_link_and_30min_ttl() {
        let body = render_password_reset_body("https://app.example.com", "tok-xyz");
        assert!(body.contains("https://app.example.com/auth/reset-password?token=tok-xyz"));
        assert!(body.contains("30 minutes"));
        assert!(body.contains("paired devices"));
    }

    #[test]
    fn render_email_change_body_includes_link_and_old_email_assurance() {
        let body = render_email_change_body("https://app.example.com", "tok-xyz");
        assert!(body.contains("https://app.example.com/auth/email-change?token=tok-xyz"));
        assert!(body.contains("old email"));
    }

    #[test]
    fn parse_smtp_url_handles_smtps_with_credentials() {
        let p = parse_smtp_url("smtps://user:pa%40ss@smtp.example.com:465").unwrap();
        assert!(p.secure);
        assert_eq!(p.host, "smtp.example.com");
        assert_eq!(p.port, 465);
        assert_eq!(p.username, "user");
        assert_eq!(p.password, "pa%40ss");
    }

    #[test]
    fn parse_smtp_url_defaults_port_for_starttls() {
        let p = parse_smtp_url("smtp://smtp.example.com").unwrap();
        assert!(!p.secure);
        assert_eq!(p.port, 587);
        assert!(p.username.is_empty());
    }

    #[test]
    fn parse_smtp_url_rejects_unknown_scheme() {
        assert!(parse_smtp_url("http://smtp.example.com").is_err());
    }
}
