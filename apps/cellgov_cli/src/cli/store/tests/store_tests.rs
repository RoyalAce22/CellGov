use super::*;

#[cfg(not(feature = "decrypt"))]
#[test]
fn a_decrypting_command_on_a_build_without_decrypt_is_refused_naming_both() {
    for command in [
        "firmware install",
        "title install",
        "title install-update",
        "self decrypt",
    ] {
        let rendered = StoreCliError::DecryptFeatureDisabled {
            command: command.to_string(),
        }
        .to_string();
        assert!(
            rendered.starts_with(&format!("`{command}`")),
            "names the command: {rendered}"
        );
        assert!(
            rendered.contains("--features decrypt"),
            "names the rebuild: {rendered}"
        );
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn a_container_label_is_the_filename_the_bar_shows() {
    use std::path::Path;

    assert_eq!(
        container_label(Path::new("/a/b/PS3UPDAT.PUP")),
        "PS3UPDAT.PUP"
    );
    assert_eq!(container_label(Path::new("game.pkg")), "game.pkg");
}
