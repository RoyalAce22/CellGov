use super::super::mutate::*;
use super::super::seeds::*;

use cellgov_ppu::loader::pt_load_segments;

fn seed(name: &str) -> Vec<u8> {
    seeds()
        .into_iter()
        .find(|seed| seed.name == name)
        .unwrap_or_else(|| panic!("no seed named {name}"))
        .bytes
}

#[test]
fn a_zero_stream_describes_an_executable_with_no_program_headers() {
    let image = structured_image(&[]);
    assert_eq!(
        pt_load_segments(&image),
        Err(cellgov_ppu::loader::LoadError::NoProgramHeaders)
    );
}

#[test]
fn corruption_stops_when_the_stream_says_zero_rounds() {
    let mut bytes = seed("prx_baseline");
    let before = bytes.clone();
    corrupt(&mut bytes, &mut FieldStream::new(&[]));
    assert_eq!(bytes, before);
    // One truncation round at position 10.
    corrupt(
        &mut bytes,
        &mut FieldStream::new(&[1, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0]),
    );
    assert_eq!(bytes.len(), 10);
}
