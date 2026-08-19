/**
 * Auth-section loading skeleton. Sized to match the centred auth card
 * layout (eyebrow + heading + two inputs + primary CTA). Server Component.
 */

const mainStyle: React.CSSProperties = {
  maxWidth: 'none',
  padding: '32px 24px 60px',
  display: 'grid',
  placeItems: 'start center',
};

const cardStyle: React.CSSProperties = {
  width: '100%',
  maxWidth: 420,
  display: 'flex',
  flexDirection: 'column',
  gap: 16,
};

export default function AuthLoading() {
  return (
    <main
      className="auth"
      style={mainStyle}
      aria-busy="true"
      aria-label="Loading"
    >
      <div style={cardStyle}>
        <div
          className="skeleton"
          style={{ height: 14, width: 80, borderRadius: 4 }}
        />
        <div
          className="skeleton"
          style={{ height: 32, width: 220, marginBottom: 8 }}
        />
        <div className="skeleton" style={{ height: 44, width: '100%' }} />
        <div className="skeleton" style={{ height: 44, width: '100%' }} />
        <div className="skeleton" style={{ height: 40, width: 120, marginTop: 8 }} />
      </div>
    </main>
  );
}
