// Browser auth transport (ADR-017). The control plane wraps /ws and most /v1
// routes in `with_auth`, which needs an identity. Browsers can set the
// `Authorization` header on `fetch` but CANNOT set request headers on a
// `WebSocket`, so the token rides as the second `Sec-WebSocket-Protocol` entry
// alongside the sentinel the server echoes back.
//
// The token is the lowercase-hex of an identity-assertion JSON. It is an
// UNSIGNED dev identity — no more trusted than the legacy `x-seasoned-hand-*`
// headers; real, verified credentials are issue #7. Keep this in sync with the
// Dioxus client (`crates/seasoned-hand-ui/src/config.rs`) and the server
// (`auth::middleware::WS_AUTH_SUBPROTOCOL`).

export const WS_AUTH_SUBPROTOCOL = "seasoned-hand-auth.v1";

function hexEncode(s: string): string {
  return Array.from(new TextEncoder().encode(s))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

export function authToken(): string {
  const fromEnv = process.env.NEXT_PUBLIC_AUTH_TOKEN;
  if (fromEnv && fromEnv.trim() !== "") return fromEnv;
  const json = JSON.stringify({
    tenant_id: "default",
    organization_id: "default",
    actor_user_id: "dev-user",
    org_role: "admin",
  });
  return hexEncode(json);
}

export function authHeaders(): Record<string, string> {
  return { Authorization: `Bearer ${authToken()}` };
}
