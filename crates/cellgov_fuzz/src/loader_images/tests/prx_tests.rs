use super::super::mutate::*;

use cellgov_ppu::prx::parse_imports;
use cellgov_ppu::sprx::parse_prx;

#[test]
fn a_short_stream_describes_a_module_the_parser_accepts() {
    // Odd selector, e_type draw 2 (ET_PRX), a three-byte name, then
    // zeros: no exports, no imports, placeholders and a parameter
    // header, no corruption.
    let stream = [1, 2, 0, 0, 0, 3, 0, 0, 0, b'a', b'b', b'c'];
    let image = structured_image(&stream);
    let prx = parse_prx(&image).unwrap();
    assert_eq!(prx.name, "abc");
    assert_eq!(prx.segment_vaddrs.len(), 4);
    assert!(prx.exports.is_empty());
    assert!(parse_imports(&image).unwrap().is_empty());
}
