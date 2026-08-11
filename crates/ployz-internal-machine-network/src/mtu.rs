use crate::{MAX_WIREGUARD_MTU, MIN_WIREGUARD_MTU, wireguard::WIREGUARD_ENCAP_OVERHEAD};

/// Detects and clamps the tunnel MTU, falling back to 1420 on failure.
pub async fn detect_mtu() -> u32 {
    let egress_mtu = match detect_egress_mtu().await {
        Ok(mtu) => mtu,
        Err(error) => {
            tracing::warn!(mtu = MAX_WIREGUARD_MTU, %error, "Failed to detect egress network MTU, falling back to the default WireGuard MTU.");
            return MAX_WIREGUARD_MTU;
        }
    };
    let mtu = egress_mtu
        .saturating_sub(WIREGUARD_ENCAP_OVERHEAD)
        .clamp(MIN_WIREGUARD_MTU, MAX_WIREGUARD_MTU);
    tracing::info!(
        mtu,
        egress_mtu,
        "Detected optimal WireGuard MTU from the egress network."
    );
    mtu
}

#[cfg(target_os = "macos")]
use crate::wireguard_darwin::detect_egress_mtu;
#[cfg(target_os = "linux")]
use crate::wireguard_linux::detect_egress_mtu;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtu_clamping_matches_constants() {
        assert_eq!(
            1_500_u32
                .saturating_sub(WIREGUARD_ENCAP_OVERHEAD)
                .clamp(MIN_WIREGUARD_MTU, MAX_WIREGUARD_MTU),
            1_420
        );
        assert_eq!(
            1_200_u32
                .saturating_sub(WIREGUARD_ENCAP_OVERHEAD)
                .clamp(MIN_WIREGUARD_MTU, MAX_WIREGUARD_MTU),
            1_280
        );
        assert_eq!(
            9_000_u32
                .saturating_sub(WIREGUARD_ENCAP_OVERHEAD)
                .clamp(MIN_WIREGUARD_MTU, MAX_WIREGUARD_MTU),
            1_420
        );
    }
}
