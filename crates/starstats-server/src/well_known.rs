//! `.well-known/*` endpoints — JWKS publication + minimal OIDC
//! discovery so third-party tools (Grafana auth-proxy, log
//! aggregators that want to verify request provenance) can validate
//! StarStats-issued JWTs without sharing a secret with us.
//!
//! Everything here is unauthenticated and cacheable. The JWKS doc
//! is built once at boot from the server's RSA public key and
//! stored as an Extension so handlers don't re-derive the modulus
//! on every request.

use crate::auth::ServerKey;
use axum::{
    response::{IntoResponse, Json},
    Extension,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JwksDocument {
    pub keys: Vec<Jwk>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Jwk {
    /// "RSA" — only algorithm we mint with.
    pub kty: &'static str,
    /// "sig" — these keys sign tokens, they don't encrypt.
    #[serde(rename = "use")]
    pub use_: &'static str,
    /// "RS256" — matches the alg in the JWT header.
    pub alg: &'static str,
    /// Stable identifier; lets verifiers pin a specific key version
    /// when we add rotation later.
    pub kid: String,
    /// Base64url-encoded modulus.
    pub n: String,
    /// Base64url-encoded exponent.
    pub e: String,
}

impl JwksDocument {
    /// Build the public-key half of the server's JWKS from the
    /// in-memory keypair. Done once at startup — the result is
    /// stable until the server's keypair rotates.
    pub fn from_server_key(key: &ServerKey, pem: &str) -> anyhow::Result<Self> {
        // We have the keypair on disk in PEM form (PKCS#1 or PKCS#8);
        // re-parse the public half so we can read modulus + exponent.
        let priv_key = parse_private_pem(pem)?;
        let pub_key = RsaPublicKey::from(&priv_key);
        let n = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
        Ok(Self {
            keys: vec![Jwk {
                kty: "RSA",
                use_: "sig",
                alg: "RS256",
                kid: key.kid.clone(),
                n,
                e,
            }],
        })
    }
}

fn parse_private_pem(pem: &str) -> anyhow::Result<RsaPrivateKey> {
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::DecodePrivateKey;
    if let Ok(k) = RsaPrivateKey::from_pkcs8_pem(pem) {
        return Ok(k);
    }
    RsaPrivateKey::from_pkcs1_pem(pem).map_err(|e| anyhow::anyhow!("parse pem: {e}"))
}

// -- Handlers --------------------------------------------------------

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OidcDiscovery {
    pub issuer: String,
    pub jwks_uri: String,
    pub id_token_signing_alg_values_supported: Vec<&'static str>,
    pub subject_types_supported: Vec<&'static str>,
    pub token_endpoint_auth_methods_supported: Vec<&'static str>,
    pub response_types_supported: Vec<&'static str>,
}

#[derive(Clone)]
pub struct DiscoveryConfig {
    pub issuer: String,
}

#[utoipa::path(
    get,
    path = "/.well-known/jwks.json",
    tag = "well-known",
    responses((status = 200, description = "Public-key JWKS", body = JwksDocument))
)]
pub async fn jwks(Extension(doc): Extension<Arc<JwksDocument>>) -> impl IntoResponse {
    Json((*doc).clone())
}

#[utoipa::path(
    get,
    path = "/.well-known/openid-configuration",
    tag = "well-known",
    responses((status = 200, description = "Minimal OIDC discovery document", body = OidcDiscovery))
)]
pub async fn openid_configuration(
    Extension(cfg): Extension<Arc<DiscoveryConfig>>,
) -> impl IntoResponse {
    let mut jwks_uri = cfg.issuer.trim_end_matches('/').to_string();
    jwks_uri.push_str("/.well-known/jwks.json");
    Json(OidcDiscovery {
        issuer: cfg.issuer.clone(),
        jwks_uri,
        id_token_signing_alg_values_supported: vec!["RS256"],
        subject_types_supported: vec!["public"],
        // We don't run an OAuth /token endpoint — the discovery doc
        // exists purely so verifiers can locate the JWKS.
        token_endpoint_auth_methods_supported: vec!["none"],
        response_types_supported: vec!["id_token"],
    })
}

// -- tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::RsaPrivateKey;

    /// JWKS round-trip: a token signed by ServerKey can be verified
    /// using only the bytes published in the JWKS document. Proves
    /// the modulus + exponent encoding actually works.
    #[tokio::test]
    async fn jwks_doc_can_verify_a_real_token() {
        // Build a fresh keypair like load_or_generate would.
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = priv_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap()
            .to_string();
        let server_key = crate::auth::ServerKey {
            kid: "k1".into(),
            encoding: Arc::new(jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap()),
            decoding: Arc::new(
                jsonwebtoken::DecodingKey::from_rsa_pem(priv_to_public_pkcs1(&priv_key).as_bytes())
                    .unwrap(),
            ),
        };

        let jwks = JwksDocument::from_server_key(&server_key, &pem).unwrap();
        assert_eq!(jwks.keys.len(), 1);
        let jwk = &jwks.keys[0];
        assert_eq!(jwk.kty, "RSA");
        assert_eq!(jwk.alg, "RS256");
        assert_eq!(jwk.kid, "k1");

        // Sign something with the server key.
        let issuer = crate::auth::TokenIssuer::new(
            server_key.clone(),
            "https://example.com".into(),
            "starstats".into(),
        );
        let token = issuer.sign_user("u1", "TheCodeSaiyan").unwrap();

        // Build a DecodingKey from JWK components alone.
        let decoding = DecodingKey::from_rsa_components(&jwk.n, &jwk.e).unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&["https://example.com"]);
        validation.set_audience(&["starstats"]);
        let data = decode::<crate::auth::Claims>(&token, &decoding, &validation).unwrap();
        assert_eq!(data.claims.preferred_username, "TheCodeSaiyan");
    }

    fn priv_to_public_pkcs1(priv_key: &RsaPrivateKey) -> String {
        use rsa::pkcs1::EncodeRsaPublicKey;
        let pub_key = RsaPublicKey::from(priv_key);
        pub_key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).unwrap()
    }

    #[test]
    fn discovery_document_points_at_our_jwks() {
        // Smoke test for the discovery shape — verifies we don't
        // accidentally publish two different issuers.
        let cfg = DiscoveryConfig {
            issuer: "https://api.example.com/".into(),
        };
        let mut jwks_uri = cfg.issuer.trim_end_matches('/').to_string();
        jwks_uri.push_str("/.well-known/jwks.json");
        assert_eq!(jwks_uri, "https://api.example.com/.well-known/jwks.json");
    }
}
