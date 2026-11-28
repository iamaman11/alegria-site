//! Native public HTTPS acquisition boundary for a future admitted Temporal
//! `web.public_page.fetch@1` owner.
//!
//! The library performs real HTTP and immutable byte persistence. It cannot
//! mint a Temporal `OwnerResultV1`, register a trusted route, or authorize an
//! operator grant. Those effects require the separate live issuer/PG fencing
//! path, so this package is deliberately not a deployable public MCP server.

pub mod archive;
pub mod transport;
