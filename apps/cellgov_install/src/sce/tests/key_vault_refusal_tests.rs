//! Which decrypt refusals the whole run answers the same.

use super::*;
use crate::keys::{KeyVaultError, Slot};

#[test]
fn a_vault_that_lacks_the_keyset_is_a_run_level_refusal() {
    let refusals = [
        SceError::Keys(Box::new(KeyVaultError::MissingSlot {
            slot: Slot::NpKlicFree,
        })),
        SceError::Keys(Box::new(KeyVaultError::MissingScepkg)),
        SceError::NoAppKey { revision: 0x0A },
        SceError::NoLv2Key {
            version: 0x0003_0055_0000_0000,
        },
        SceError::NoNpdrmKey { revision: 0x0A },
        SceError::RapPboxNotAPermutation { index: 3 },
    ];
    for e in &refusals {
        assert!(e.is_key_vault_refusal(), "{e}");
    }
}

#[test]
fn a_refusal_that_names_the_image_or_its_rap_is_not_a_run_level_refusal() {
    let image_side = [
        SceError::KeyEnvelopePadding,
        SceError::AesCbcDecryptFailed,
        SceError::NoCandidateOpensEnvelope {
            class: "APP",
            revision: 0x0A,
            tried: 2,
        },
        SceError::NoRapForNpdrmTitle {
            content_id: "UP0001-CGOV00001_00-TESTTESTTESTTEST".into(),
        },
        SceError::RapRead {
            content_id: "UP0001-CGOV00001_00-TESTTESTTESTTEST".into(),
            source: crate::npdrm::RapReadError::Missing {
                path: std::path::PathBuf::from("UP0001-CGOV00001_00-TESTTESTTESTTEST.rap"),
            },
        },
        SceError::DecryptFeatureDisabled,
        SceError::DebugSelfUnsupported {
            revision_flags: 0x8001,
        },
        SceError::BadMagic { got: 0 },
    ];
    for e in &image_side {
        assert!(!e.is_key_vault_refusal(), "{e}");
    }
}
