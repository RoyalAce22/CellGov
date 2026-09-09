//! The sample documents against the paths the store writes.

use super::schema;

fn status() -> serde_json::Value {
    let rendered = schema::render();
    let block = rendered
        .split("```json\n")
        .nth(1)
        .and_then(|rest| rest.split_once("\n```"))
        .expect("the status sample is the first fenced block")
        .0;
    serde_json::from_str(block).expect("the status sample is JSON")
}

#[test]
fn the_firmware_sample_names_the_record_and_entry_the_store_files() {
    let doc = status();
    let firmware = &doc["firmware"][0];
    assert_eq!(firmware["entry_dir"], "firmware/4.93");
    assert_eq!(
        firmware["record"],
        ".cellgov/installs/firmware/4.93.install.toml"
    );
}

#[test]
fn the_title_sample_names_the_records_the_store_files() {
    let doc = status();
    let title = &doc["titles"][0];
    assert_eq!(
        title["base"]["record"],
        ".cellgov/installs/titles/NPUA80001/base.install.toml"
    );
    assert_eq!(
        title["updates"][0]["record"],
        ".cellgov/installs/titles/NPUA80001/update-1.02.install.toml"
    );
}

#[test]
fn the_title_sample_names_the_trees_the_installers_write() {
    let doc = status();
    let title = &doc["titles"][0];
    assert_eq!(title["base"]["dir"], "dev_hdd0/game/NPUA80001");
    assert_eq!(
        title["updates"][0]["dir"],
        "titles/NPUA80001/updates/1.02/game"
    );
}
