//! Network ports shared by Ployz machine services and their clients.

/// TCP port for the Machine API on the management WireGuard network.
pub const MACHINE_API_PORT: u16 = 51_000;

/// TCP port for the embedded container registry on a machine's cluster address.
pub const EMBEDDED_REGISTRY_PORT: u16 = 51_500;
