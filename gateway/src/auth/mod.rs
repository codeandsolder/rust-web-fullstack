//! Authentication: JWT creation, validation, middleware, and handlers.
//!
//! This module has been split from the original monolithic `auth.rs` into
//! focused submodules:
//!
//! * [`error`] — [`AppError`] enum with HTTP response mapping
//! * [`jwt`] — `EdDSA` JWT creation and validation via `jsonwebtoken`
//! * [`middleware`] — axum middleware for Bearer token validation
//! * [`handlers`] — login, refresh, logout, and protected route handlers

pub mod error;
pub mod handlers;
pub mod jwt;
pub mod middleware;

pub use self::error::AppError;
pub use self::handlers::{login_handler, logout_handler, protected_handler, refresh_handler};
pub use self::jwt::{Claims, create_jwt, validate_jwt};
pub use self::middleware::auth_middleware;
