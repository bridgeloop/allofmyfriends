// aiden@cmp.bz
// base64 encoder that doesn't use cringe operators like div and mod
// the optimiser would probably generate code similar to this anyway though
// i was just bored lol

const TABLE: [u8; 64] = *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode_output_size(count: u32) -> usize {
	return if count == 0 {
		 0
	} else {
		// code is equiv to: round_up(count, multiple of 3) / 3 * 4
		((count as usize + 2) * 0x55555556) >> 30 & !3
	}
}
pub fn encode(input: &[u8]) -> Result<Vec<u8>, ()> {
	let len: u32 = input.len().try_into().map_err(|_| ())?;
	let mut iter = input.into_iter();

	let mut prev = 0u8;
	let mut remainder = 0u8;
	let mut read_6_bits = || {
		macro_rules! set_remainder {
			($val: expr) => {
				remainder = $val;
				prev = prev.wrapping_add(1);
			};
		}

		let prev_bits = (prev & 0b11) * 2;

		let req = 6 - prev_bits;
		if req == 0 {
			let six_bits = remainder;
			set_remainder!(0);
			return Some(six_bits);
		}

		let curr_byte = iter.next()?;

		let consumed = curr_byte >> (8 - req);
		let six_bits = (remainder << (6 - prev_bits)) | consumed;
		set_remainder!(curr_byte & (0xff >> req));

		return Some(six_bits);
	};

	let mut output = Vec::with_capacity(encode_output_size(len));

	while let Some(idx) = read_6_bits() {
		output.push(TABLE[idx as usize]);
	}
	let prev = (prev & 0b11) * 2;
	if prev != 0 {
		let req = 6 - prev;
		let idx = remainder << req;
		output.push(TABLE[idx as usize]);
	}
	output.resize(output.capacity(), b'=');

	return Ok(output);
}