use ployz_scripts::INSTALL_SCRIPT;

const ORACLE_INSTALL_SCRIPT: &str = include_str!("../../../upstream/uncloud/scripts/install.sh");

fn renamed_oracle() -> String {
    ORACLE_INSTALL_SCRIPT
        .replace("UNCLOUD", "PLOYZ")
        .replace("Uncloud", "Ployz")
        .replace("uncloud", "ployz")
        .replace("psviderski/ployz", "getployz/ployz")
        .replace(
            "'uc deploy' or 'uc image push'",
            "'ployz deploy' or 'ployz image push'",
        )
        .replace("create a uc alias", "create a ployz alias")
}

#[test]
fn embedded_script_matches_the_renamed_oracle_byte_for_byte() {
    assert_eq!(INSTALL_SCRIPT, renamed_oracle());
}

#[test]
fn embedded_script_is_complete_and_has_no_old_product_name() {
    assert!(INSTALL_SCRIPT.starts_with("#!/usr/bin/env bash\n\nset -euo pipefail\n"));
    assert!(
        INSTALL_SCRIPT.ends_with("log \"✓ Ployz installed on the machine successfully! 🎉\"\n")
    );
    assert!(!INSTALL_SCRIPT.to_ascii_lowercase().contains("uncloud"));
    assert!(!INSTALL_SCRIPT.contains(" uc "));
    assert!(!INSTALL_SCRIPT.contains("psviderski"));
}

#[test]
fn embedded_script_exposes_the_caller_facing_configuration_names() {
    for expected in [
        "PLOYZ_VERSION",
        "PLOYZ_GROUP_ADD_USER",
        "PLOYZ_DATA_DIR",
        "ployzd",
        "ployz.service",
        "ployz-uninstall",
        "https://raw.githubusercontent.com/getployz/ployz/${uninstall_ref}/scripts/uninstall.sh",
    ] {
        assert!(
            INSTALL_SCRIPT.contains(expected),
            "missing expected install contract: {expected}"
        );
    }
}
