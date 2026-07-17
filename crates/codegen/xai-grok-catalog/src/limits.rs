//! Locked numeric bounds for dynamic provider configuration and discovery.
//!
//! These constants are part of the security posture of dynamic providers:
//! they cap identifier/endpoint lengths, discovery response sizes, and
//! network timeouts. Changing any of them is a deliberate, reviewed decision;
//! tests in [`crate::dynamic`] lock the exact values.

use std::time::Duration;

pub use crate::types::{MAX_MODEL_ID_BYTES, MAX_PROVIDER_ID_BYTES};

/// Maximum accepted size of a model-discovery response body, in bytes.
pub const MAX_DISCOVERY_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Maximum number of models accepted from a single discovery response.
pub const MAX_DISCOVERED_MODELS: usize = 2_000;

/// Maximum byte length of a dynamic provider display name.
pub const MAX_PROVIDER_NAME_BYTES: usize = 128;

/// Maximum byte length of a model display name.
pub const MAX_MODEL_NAME_BYTES: usize = 512;

/// Maximum byte length of a base URL or endpoint override.
pub const MAX_ENDPOINT_BYTES: usize = 2_048;

/// Maximum number of HTTP redirects followed during discovery.
pub const MAX_REDIRECTS: usize = 5;

/// Timeout for a model-discovery request.
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for a provider health-check request.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);

/// Maximum age of a cached dynamic discovery result before it is stale.
pub const DYNAMIC_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
