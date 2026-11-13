// lzss.rs — LZSS decompressor matching the Eushully AGE engine variant.
//
// Parameters (from Kelebek1's Python implementation and GARbro C# source):
//   - Ring buffer size: 4096 bytes (12-bit offset)
//   - Min match length: 3 bytes
//   - Max match length: 18 bytes (4-bit length field, value + 3)
//   - Initial buffer fill: 0x00 (configurable)
//   - Flag byte: 8 flags per byte, LSB first (bit 0 = first token)
//   - Literal: flag bit = 1
//   - Back-reference: flag bit = 0
//
// Back-reference encoding (little-endian 16-bit):
//   low byte  = low 8 bits of offset
//   high byte = high 4 bits of offset (bits 4-7) | (length - 3) (bits 0-3)

pub fn decompress(input: &[u8], initial_fill: u8) -> Result<Vec<u8>, String> {
    const RING_SIZE: usize = 4096;
    const MIN_MATCH: usize = 3;
    const MAX_MATCH: usize = 18;

    let mut ring = vec![initial_fill; RING_SIZE];
    let mut ring_pos: usize = RING_SIZE - MAX_MATCH; // matches GARbro's initial position

    let mut output: Vec<u8> = Vec::with_capacity(input.len() * 3);
    let mut src = 0usize;

    while src < input.len() {
        let flags = input[src] as u32;
        src += 1;

        for bit in 0..8 {
            if src >= input.len() {
                break;
            }

            if (flags >> bit) & 1 == 1 {
                // Literal byte
                let byte = input[src];
                src += 1;
                output.push(byte);
                ring[ring_pos] = byte;
                ring_pos = (ring_pos + 1) & (RING_SIZE - 1);
            } else {
                // Back-reference: 2 bytes
                if src + 1 >= input.len() {
                    break;
                }
                let lo = input[src] as usize;
                let hi = input[src + 1] as usize;
                src += 2;

                let offset = lo | ((hi & 0xF0) << 4);
                let length = (hi & 0x0F) + MIN_MATCH;

                let mut copy_pos = offset;
                for _ in 0..length {
                    let byte = ring[copy_pos & (RING_SIZE - 1)];
                    output.push(byte);
                    ring[ring_pos] = byte;
                    ring_pos = (ring_pos + 1) & (RING_SIZE - 1);
                    copy_pos += 1;
                }
            }
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_empty() {
        let result = decompress(&[], 0x00).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn decompress_all_literals() {
        // Flag byte = 0xFF (all literals), followed by 8 literal bytes
        let input = vec![0xFF, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48];
        let result = decompress(&input, 0x00).unwrap();
        assert_eq!(&result, b"ABCDEFGH");
    }
}
