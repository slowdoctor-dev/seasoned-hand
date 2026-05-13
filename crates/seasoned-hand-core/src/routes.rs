//! Shared HTTP-route outcome type used by `verifier::routes` and
//! `checkpoint::routes`. Lives in core so the route functions can stay
//! pure (no axum dep); the server crate adapts each variant to an
//! axum response via its own `render_outcome` helper.

#[derive(Debug)]
pub enum RouteOutcome<T> {
    Ok(T),
    NotFound(String),
    Internal(String),
}
