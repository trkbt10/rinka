//! Capture verification without an image dependency.

use crate::error::HarnessError;
use std::path::Path;

/// The eight-byte PNG file signature.
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Decodes a PNG file's header and returns its pixel dimensions.
///
/// Screenshot acceptance requires proving the capture is a decodable PNG of
/// non-trivial size; the signature plus the mandatory leading `IHDR` chunk
/// carry exactly that, so no image library enters the dependency graph.
pub fn png_dimensions(path: &Path) -> Result<(u32, u32), HarnessError> {
    let capture_error = |reason: String| HarnessError::Capture {
        path: path.to_path_buf(),
        reason,
    };
    let bytes = std::fs::read(path).map_err(|error| capture_error(error.to_string()))?;
    if bytes.len() < 33 {
        return Err(capture_error(format!(
            "file holds {} bytes, shorter than a minimal PNG header",
            bytes.len()
        )));
    }
    if bytes[..8] != PNG_SIGNATURE {
        return Err(capture_error("missing PNG signature".to_owned()));
    }
    // The first chunk of a valid PNG is IHDR: 4-byte length (13), the type
    // "IHDR", then width and height as big-endian 32-bit integers.
    if &bytes[12..16] != b"IHDR" {
        return Err(capture_error("first chunk is not IHDR".to_owned()));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width == 0 || height == 0 {
        return Err(capture_error(format!(
            "decoded trivial dimensions {width}x{height}"
        )));
    }
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        // Bit depth, color type, compression, filter, interlace, CRC.
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn png_dimensions_reads_the_ihdr_geometry() {
        let path = std::env::temp_dir().join(format!(
            "rinka-test-png-dimensions-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, minimal_png(640, 480)).expect("write fixture");
        assert_eq!(png_dimensions(&path).expect("decodes"), (640, 480));
        std::fs::remove_file(&path).expect("fixture cleanup");
    }

    #[test]
    fn png_dimensions_rejects_trivial_and_non_png_content() {
        let path =
            std::env::temp_dir().join(format!("rinka-test-png-reject-{}.png", std::process::id()));
        std::fs::write(&path, minimal_png(0, 480)).expect("write fixture");
        assert!(matches!(
            png_dimensions(&path),
            Err(HarnessError::Capture { .. })
        ));
        std::fs::write(&path, b"not a png at all").expect("write fixture");
        assert!(matches!(
            png_dimensions(&path),
            Err(HarnessError::Capture { .. })
        ));
        std::fs::remove_file(&path).expect("fixture cleanup");
    }
}
