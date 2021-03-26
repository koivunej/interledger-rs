#![forbid(unsafe_code)]

use std::convert::TryFrom;
use std::fmt::Write;
use std::io::{Error, ErrorKind, Result};
use std::u64;

use byteorder::{BigEndian, ReadBytesExt};
use bytes::{Buf, BufMut, BytesMut};
use chrono::{TimeZone, Utc};

const HIGH_BIT: u8 = 0x80;
const LOWER_SEVEN_BITS: u8 = 0x7f;
static GENERALIZED_TIME_FORMAT: &str = "%Y%m%d%H%M%S%.3fZ";

/// Returns the size (in bytes) of the buffer that encodes a VarOctetString of
/// `length` bytes.
pub fn predict_var_octet_string(length: usize) -> usize {
    if length < 128 {
        1 + length
    } else {
        let length_of_length = predict_var_uint_size(length as u64) as usize;
        1 + length_of_length + length
    }
}

/// Returns the minimum number of bytes needed to encode the `value` without leading zeroes in
/// big-endian representation.
pub fn predict_var_uint_size(value: u64) -> u8 {
    // avoid branching on zero by always orring one; it will not have any effect on other inputs
    let value = value | 1;

    // this will be now be 0..=63 as we handled the zero away
    let highest_bit = 64 - value.leading_zeros();

    // do a highest_bit.ceiling_div(8)
    ((highest_bit + 8 - 1) / 8) as u8
}

pub fn extract_var_octet_string(mut buffer: BytesMut) -> Result<BytesMut> {
    let buffer_length = buffer.len();
    let mut reader = &buffer[..];
    let content_length = reader.read_var_octet_string_length()?;
    let content_offset = buffer_length - reader.len();

    let mut remaining = buffer.split_off(content_offset);
    if remaining.len() < content_length {
        Err(Error::new(ErrorKind::UnexpectedEof, "buffer too small"))
    } else {
        Ok(remaining.split_to(content_length))
    }
}

pub trait BufOerExt<'a> {
    fn peek_var_octet_string(&self) -> Result<&'a [u8]>;
    fn read_var_octet_string(&mut self) -> Result<&'a [u8]>;
    fn skip(&mut self, discard_bytes: usize) -> Result<()>;
    fn skip_var_octet_string(&mut self) -> Result<()>;
    fn read_var_octet_string_length(&mut self) -> Result<usize>;
    fn read_var_uint(&mut self) -> Result<u64>;

    /// Decodes a variable length timestamp according to [RFC-0030].
    ///
    /// [RFC-0030]: https://github.com/interledger/rfcs/blob/2473d2963a65e5534076c483f3c08a81b8e0cc88/0030-notes-on-oer-encoding/0030-notes-on-oer-encoding.md#variable-length-timestamps
    fn read_variable_length_timestamp(&mut self) -> Result<VariableLengthTimestamp>;
}

impl<'a> BufOerExt<'a> for &'a [u8] {
    /// Decodes variable-length octet string buffer without moving the cursor.
    #[inline]
    fn peek_var_octet_string(&self) -> Result<&'a [u8]> {
        let mut peek = &self[..];
        let actual_length = peek.read_var_octet_string_length()?;
        let offset = self.len() - peek.len();
        if peek.len() < actual_length {
            Err(Error::new(ErrorKind::UnexpectedEof, "buffer too small"))
        } else {
            Ok(&self[offset..(offset + actual_length)])
        }
    }

    /// Decodes variable-length octet string.
    #[inline]
    fn read_var_octet_string(&mut self) -> Result<&'a [u8]> {
        let actual_length = self.read_var_octet_string_length()?;
        if self.len() < actual_length {
            Err(Error::new(ErrorKind::UnexpectedEof, "buffer too small"))
        } else {
            let to_return = &self[..actual_length];
            *self = &self[actual_length..];
            Ok(to_return)
        }
    }

    #[inline]
    fn skip(&mut self, discard_bytes: usize) -> Result<()> {
        if self.len() < discard_bytes {
            Err(Error::new(ErrorKind::UnexpectedEof, "buffer too small"))
        } else {
            *self = &self[discard_bytes..];
            Ok(())
        }
    }

    #[inline]
    fn skip_var_octet_string(&mut self) -> Result<()> {
        let actual_length = self.read_var_octet_string_length()?;
        self.skip(actual_length)
    }

    #[doc(hidden)]
    #[inline]
    fn read_var_octet_string_length(&mut self) -> Result<usize> {
        let length = self.read_u8()?;
        if length & HIGH_BIT != 0 {
            let length_prefix_length = (length & LOWER_SEVEN_BITS) as usize;
            // TODO check for canonical length
            if length_prefix_length > 8 {
                Err(Error::new(
                    ErrorKind::InvalidData,
                    "length prefix too large",
                ))
            } else if length_prefix_length == 0 {
                Err(Error::new(
                    ErrorKind::InvalidData,
                    "indefinite lengths are not allowed",
                ))
            } else {
                let uint = self.read_uint::<BigEndian>(length_prefix_length)?;

                check_no_leading_zeroes(length_prefix_length, uint)?;

                if length_prefix_length == 1 && uint < 128 {
                    // this doesn't apply to the var uint which is at minimum two octets (0x00,
                    // 0x00) but var len octet strings.
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "variable length prefix with unnecessary multibyte length",
                    ));
                }

                // it makes no sense for a length to be u64 but usize, and even that is quite a lot
                let uint = usize::try_from(uint).map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "var octet length overflow",
                    )
                })?;

                Ok(uint)
            }
        } else {
            Ok(length as usize)
        }
    }

    /// Decodes variable-length octet unsigned integer to get `u64`.
    #[inline]
    fn read_var_uint(&mut self) -> Result<u64> {
        let size = self.read_var_octet_string_length()?;
        if size == 0 {
            Err(Error::new(ErrorKind::InvalidData, "zero-length VarUInt"))
        } else if size > 8 {
            Err(Error::new(ErrorKind::InvalidData, "VarUInt too large"))
        } else {
            let uint = self.read_uint::<BigEndian>(size)?;

            check_no_leading_zeroes(size, uint)?;

            Ok(uint)
        }
    }

    fn read_variable_length_timestamp(&mut self) -> Result<VariableLengthTimestamp> {
        let regex = regex::bytes::Regex::new(r"^[0-9]{4}[0-9]{2}{5}(\.[0-9]{1,3})?$").unwrap();

        // This takes the first byte as the length
        let octets = self.read_var_octet_string()?;

        if regex.is_match(octets) {
            // return some Err()
        }

        // the string might still have bad date in it
        let s = std::str::from_utf8(octets).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Octets to str conversion err: {}", e),
            )
        })?;

        let ts = Utc
            .datetime_from_str(s, GENERALIZED_TIME_FORMAT)
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("UTC datatime conversion err: {}", e),
                )
            })?;

        Ok(VariableLengthTimestamp {
            inner: ts,
            len: SmallVariableLengthField::from(octets.len()),
        })
    }
}

pub trait SmallVariableLengthField {
    fn from(len: usize) -> Self;
    fn to_usize(&self) -> usize;
    fn trim_millis(&self, ts: &chrono::DateTime<chrono::Utc>) -> String;
}

impl SmallVariableLengthField for u8 {
    fn from(len: usize) -> Self {
        u8::try_from(len).unwrap()
    }

    fn to_usize(&self) -> usize {
        usize::try_from(*self).unwrap()
    }

    fn trim_millis(&self, ts: &chrono::DateTime<chrono::Utc>) -> String {
        let str = ts.format(GENERALIZED_TIME_FORMAT).to_string();
        let mut str_trimmed = match self {
            15 => str[..14].to_owned(),
            16 => panic!("Should not have time at this length"),
            17 => str[..16].to_owned(),
            18 => str[..17].to_owned(),
            19 => return str,
            _ => panic!("Should not have time at this length"),
        };
        str_trimmed.push('Z');
        str_trimmed
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct VariableLengthTimestamp {
    pub inner: chrono::DateTime<chrono::Utc>,
    pub len: u8,
}

impl VariableLengthTimestamp {
    fn trim(&self) -> String {
        let delayed_format = self.inner.format(GENERALIZED_TIME_FORMAT);
        let mut s = String::with_capacity(self.len as usize);
        // TODO: handle error
        write!(&mut s, "{}", delayed_format).unwrap();
        let s = match self.len {
            15 => {
                // when parsing there were no fractions
                &s[..14]
            }
            17 => {
                // %.1f
                &s[..16]
            }
            18 => {
                // %.2f
                &s[..17]
            }
            19 => {
                // original %.3f is good
                return s;
            }
            x => unreachable!("Should not have timestamp of length: {}", x),
        };

        // there is probably some nifty helper in std for this but
        let mut out = String::with_capacity(s.len() + 1);
        out.push_str(s);
        out.push('Z');
        out
    }
}

fn check_no_leading_zeroes(_size_on_wire: usize, _uint: u64) -> Result<()> {
    #[cfg(feature = "strict")]
    if _size_on_wire != predict_var_uint_size(_uint) as usize {
        // if we dont check for this fuzzing roundtrip tests will break, as there are
        // "128 | 1, x" class of inputs, where "x" = 0..128
        return Err(Error::new(
            ErrorKind::InvalidData,
            "variable length prefix with leading zeros",
        ));
    }
    Ok(())
}

pub trait MutBufOerExt: BufMut + Sized {
    /// Encodes bytes as variable-length octet encoded string and puts it into `Buf`.
    #[inline]
    fn put_var_octet_string<B: Buf>(&mut self, buf: B) {
        self.put_var_octet_string_length(buf.remaining());
        self.put(buf);
    }

    /// Encodes the length of a datatype as variable-length octet encoded unsigned integer and puts it into `Buf`
    #[inline]
    fn put_var_octet_string_length(&mut self, length: usize) {
        if length < 128 {
            self.put_u8(length as u8);
        } else {
            let length_of_length = predict_var_uint_size(length as u64) as usize;
            self.put_u8(HIGH_BIT | length_of_length as u8);
            self.put_uint(length as u64, length_of_length);
        }
    }

    /// Encodes `u64` as variable-length octet encoded unsigned integer and puts it into `BufMut`
    #[inline]
    fn put_var_uint(&mut self, uint: u64) {
        let size = predict_var_uint_size(uint) as usize;
        self.put_var_octet_string_length(size);
        self.put_uint(uint, size);
    }

    /// Encodes the given timestamp per the rules, see
    /// [`BufOerExt::read_variable_length_timestamp`].
    fn put_variable_length_timestamp(&mut self, vts: &VariableLengthTimestamp) {
        self.put_var_octet_string(vts.trim().as_bytes());
    }
}

impl<B: BufMut + Sized> MutBufOerExt for B {}

#[cfg(test)]
mod test_functions {
    use bytes::BytesMut;

    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_predict_var_octet_string() {
        let zeroes = &[0; 4096][..];
        let mut buffer = BytesMut::with_capacity(5000);
        for i in 0..4096 {
            buffer.clear();
            buffer.put_var_octet_string(&zeroes[..i]);
            assert_eq!(
                predict_var_octet_string(i),
                buffer.len(),
                "string_length={:?}",
                i,
            );
        }
    }

    #[test]
    fn test_predict_var_uint_size() {
        assert_eq!(predict_var_uint_size(0), 1);
        assert_eq!(predict_var_uint_size(1), 1);
        assert_eq!(predict_var_uint_size(0xff), 1);
        assert_eq!(predict_var_uint_size(0xff + 1), 2);
        assert_eq!(predict_var_uint_size(u64::MAX - 1), 8);
        assert_eq!(predict_var_uint_size(u64::MAX), 8);
    }

    #[test]
    fn test_extract_var_octet_string() {
        assert_eq!(
            extract_var_octet_string(BytesMut::from(TWO_BYTE_VARSTR)).unwrap(),
            BytesMut::from(&TWO_BYTE_VARSTR[1..3]),
        );
        assert_eq!(
            extract_var_octet_string(BytesMut::new())
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof,
        );
        assert_eq!(
            extract_var_octet_string(BytesMut::from(LENGTH_TOO_HIGH_VARSTR))
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof,
        );
    }
}

#[cfg(test)]
mod test_buf_oer_ext {
    use once_cell::sync::Lazy;

    use super::fixtures::*;
    use super::*;

    // These bufferes have their lengths encoded in multiple bytes.
    static SIZE_128_VARSTR: Lazy<Vec<u8>> = Lazy::new(|| {
        let mut data = vec![0x81, 0x80];
        data.extend(&[0x00; 128][..]);
        data
    });
    static SIZE_5678_VARSTR: Lazy<Vec<u8>> = Lazy::new(|| {
        let mut data = vec![0x82, 0x16, 0x2e];
        data.extend(&[0x00; 5678][..]);
        data
    });

    #[test]
    fn test_peek_var_octet_string() {
        let tests: &[(&[u8], &[u8])] = &[
            (&[0x00], &[]),
            (&ZERO_LENGTH_VARSTR, &[]),
            (&ONE_BYTE_VARSTR, &[0x01]),
            (&TWO_BYTE_VARSTR, &[0x01, 0x02]),
            (&SIZE_128_VARSTR, &[0; 128][..]),
            (&SIZE_5678_VARSTR, &[0; 5678][..]),
        ];
        for (buffer, varstr) in tests {
            assert_eq!((&buffer[..]).peek_var_octet_string().unwrap(), *varstr,);
        }

        assert_eq!(
            (&[][..]).peek_var_octet_string().unwrap_err().kind(),
            ErrorKind::UnexpectedEof,
        );
        assert_eq!(
            LENGTH_TOO_HIGH_VARSTR
                .peek_var_octet_string()
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof,
        );
    }

    #[test]
    fn test_skip() {
        let mut empty = &[][..];
        assert_eq!(empty.skip(1).unwrap_err().kind(), ErrorKind::UnexpectedEof,);

        let mut reader = &[0x01, 0x02, 0x03][..];
        assert!(reader.skip(2).is_ok());
        assert_eq!(reader, &[0x03]);
    }

    #[test]
    fn test_skip_var_octet_string() {
        let tests: &[(Vec<u8>, usize)] = &[
            (ZERO_LENGTH_VARSTR.to_vec(), 1),
            (ONE_BYTE_VARSTR.to_vec(), 2),
            (TWO_BYTE_VARSTR.to_vec(), 3),
            (SIZE_128_VARSTR.clone(), SIZE_128_VARSTR.len()),
            (SIZE_5678_VARSTR.clone(), SIZE_5678_VARSTR.len()),
        ];
        for (buffer, offset) in tests {
            let mut reader = &buffer[..];
            assert!(reader.skip_var_octet_string().is_ok());
            assert_eq!(reader.len(), buffer.len() - *offset);
        }

        assert_eq!(
            (&[][..]).skip_var_octet_string().unwrap_err().kind(),
            ErrorKind::UnexpectedEof,
        );

        assert_eq!(
            (&LENGTH_TOO_HIGH_VARSTR[..])
                .skip_var_octet_string()
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof,
        );
    }

    #[test]
    fn read_too_big_var_octet_string_length() {
        // The length of the octet string fits in 9 bytes, which is too big.
        let mut too_big: &[u8] = &[HIGH_BIT | 0x09];
        assert_eq!(
            too_big.read_var_octet_string_length().unwrap_err().kind(),
            ErrorKind::InvalidData,
        );
    }

    #[test]
    fn read_empty_octet_string() {
        let mut reader = &[0][..];
        let slice = reader.read_var_octet_string().unwrap();
        assert!(slice.is_empty());
    }

    #[test]
    fn read_smaller_octet_string() {
        let mut two_bytes = &[0x01, 0xb0][..];
        assert_eq!(two_bytes.read_var_octet_string().unwrap(), &[0xb0]);
    }

    #[test]
    fn read_longer_length_octet_string() {
        let mut larger = vec![0x82, 0x01, 0x00];
        let larger_string = [0xb0; 256];
        larger.extend(&larger_string);

        let mut reader = &larger[..];

        assert_eq!(reader.read_var_octet_string().unwrap(), &larger_string[..]);
    }

    #[test]
    fn indefinite_len_octets() {
        #[allow(clippy::identity_op)]
        let indefinite = HIGH_BIT | 0x00;

        let mut bytes = &[indefinite, 0x00, 0x01, 0x02][..];

        let e = bytes.read_var_octet_string_length().unwrap_err();

        assert_eq!(e.kind(), ErrorKind::InvalidData);
        assert_eq!("indefinite lengths are not allowed", e.to_string());
    }

    #[test]
    fn way_too_long_octets_with_126_bytes_of_length() {
        // this would be quite the long string, not great for network programming
        let mut bytes = vec![HIGH_BIT | 126];
        bytes.extend(std::iter::repeat(0xff).take(125));
        bytes.push(1);

        let mut reader = &bytes[..];

        let e = reader.read_var_octet_string_length().unwrap_err();

        assert_eq!("length prefix too large", e.to_string());
    }

    #[test]
    fn way_too_long_octets_with_9_bytes_of_length() {
        let mut bytes = vec![HIGH_BIT | 9];
        bytes.extend(std::iter::repeat(0xff).take(8));
        bytes.push(1);

        let mut reader = &bytes[..];

        let e = reader.read_var_octet_string_length().unwrap_err();

        assert_eq!("length prefix too large", e.to_string());
    }

    #[test]
    fn max_len_octets() {
        let mut bytes = vec![HIGH_BIT | 4];
        bytes.extend(std::iter::repeat(0xff).take(4));

        // here 84 ff ff ff ff is used as a max usize (in 32-bit systems)

        let mut reader = &bytes[..];

        let len = reader.read_var_octet_string_length().unwrap();

        assert_eq!(u32::MAX, len as u32);
    }

    #[test]
    fn test_read_var_uint() {
        let tests: &[(&[u8], u64, usize)] = &[
            (&[0x01, 0x00], 0, 2),
            (&[0x01, 0x09], 9, 2),
            (&[0x02, 0x01, 0x02], 0x0102, 3),
            (&[0x03, 0x01, 0x02, 0x03], 0x0001_0203, 4),
            (&[0x04, 0x01, 0x02, 0x03, 0x04], 0x0102_0304, 5),
            (&[0x05, 0x01, 0x02, 0x03, 0x04, 0x05], 0x0001_0203_0405, 6),
            (
                &[0x06, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
                0x0102_0304_0506,
                7,
            ),
            (
                &[0x07, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
                0x0001_0203_0405_0607,
                8,
            ),
            (
                &[0x08, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
                0x0102_0304_0506_0708,
                9,
            ),
        ];

        for (buffer, value, offset) in tests {
            let mut reader = &buffer[..];
            let uint = reader.read_var_uint().unwrap();
            assert_eq!(uint, *value);
            assert_eq!(reader.len(), buffer.len() - *offset);
        }

        let tests: &[(&[u8], ErrorKind)] = &[
            // VarUint's must have at least 1 byte.
            (&[0x00], ErrorKind::InvalidData),
            // Data must be present.
            (&[0x04], ErrorKind::UnexpectedEof),
            // Enough bytes must be present.
            (&[0x04, 0x01, 0x02, 0x03], ErrorKind::UnexpectedEof),
            // Too many bytes.
            (
                &[0x09, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09],
                ErrorKind::InvalidData,
            ),
        ];

        for (buffer, error_kind) in tests {
            assert_eq!(
                (&buffer[..]).read_var_uint().unwrap_err().kind(),
                *error_kind,
            );
        }
    }

    #[test]
    fn peek_too_long_uint() {
        // in interledger-stream there is a use case to accept larger than u64::MAX for a varuint.
        // it is done through peeking for the length of the varuint, using the length > 8 to
        // default to u64::MAX.
        let bytes: &[u8] = &[0x09u8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];

        let reader = bytes;
        assert_eq!(9, reader.peek_var_octet_string().unwrap().len());
    }

    #[test]
    fn read_variable_length_timestamp() {
        let valid: &[(&[u8], &str)] = &[
            (b"20171224161432.279Z", "2017-12-24 16:14:32.279 UTC"),
            (b"20171224161432.27Z", "2017-12-24 16:14:32.270 UTC"),
            (b"20171224161432.2Z", "2017-12-24 16:14:32.200 UTC"),
            (b"20171224161432Z", "2017-12-24 16:14:32 UTC"),
            // (b"20171224161432.279Z", "2017-12-24T16:14:32.279Z"),
            // (b"20171224161432.27Z", "2017-12-24T16:14:32.270Z"),
            // (b"20171224161432.2Z", "2017-12-24T16:14:32.200Z"),
            // (b"20171224161432Z", "2017-12-24T16:14:32.000Z"),
            // (b"20161231235960.852Z", "2016-12-31T23:59:60.852Z"),
            // (b"20171225000000Z", "2017-12-25T00:00:00.000Z"),
            // (b"99991224161432.279Z", "9999-12-24T16:14:32.279Z"),
        ];

        let mut buffer = BytesMut::with_capacity(1 + valid[0].0.len());

        for &(input, expected) in valid {
            buffer.clear();
            buffer.put_var_octet_string(input);
            let ts = buffer.as_ref().read_variable_length_timestamp().unwrap();
            assert_eq!(ts.len as usize, input.len());
            assert_eq!(expected, ts.inner.to_string());
        }
    }
}

#[cfg(test)]
mod buf_mut_oer_ext {
    use super::*;

    #[test]
    fn test_put_var_octet_string() {
        let long_varstr = &[0x00; 256][..];
        let long_buffer = {
            let mut buffer = vec![0x82, 0x01, 0x00];
            buffer.extend_from_slice(&long_varstr);
            buffer
        };

        let tests: &[(&[u8], &[u8])] = &[
            // Write an empty string.
            (b"", b"\x00"),
            // Write a single byte.
            (b"\xb0", b"\x01\xb0"),
            (long_varstr, &long_buffer[..]),
        ];

        let mut writer = BytesMut::with_capacity(10);

        for (varstr, buffer) in tests {
            writer.clear();
            writer.put_var_octet_string(*varstr);
            assert_eq!(&writer[..], *buffer);
        }
    }

    #[test]
    fn test_put_var_uint() {
        let tests: &[(&[u8], u64, usize)] = &[
            (&[0x01, 0x00], 0, 2),
            (&[0x01, 0x09], 9, 2),
            (&[0x02, 0x01, 0x02], 0x0102, 3),
            (&[0x03, 0x01, 0x02, 0x03], 0x0001_0203, 4),
            (&[0x04, 0x01, 0x02, 0x03, 0x04], 0x0102_0304, 5),
            (&[0x05, 0x01, 0x02, 0x03, 0x04, 0x05], 0x0001_0203_0405, 6),
            (
                &[0x06, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
                0x0102_0304_0506,
                7,
            ),
            (
                &[0x07, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
                0x0001_0203_0405_0607,
                8,
            ),
            (
                &[0x08, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
                0x0102_0304_0506_0708,
                9,
            ),
        ];

        let mut writer = BytesMut::with_capacity(10);

        for (buffer, value, _offset) in tests {
            writer.clear();
            writer.put_var_uint(*value);
            assert_eq!(writer, *buffer);
        }
    }

    #[test]
    fn test_put_variable_length_timestamp() {
        let tests: &[(&[u8], &str, u8)] = &[
            (b"20171224161432.279Z", "20171224161432.279Z", 19),
            (b"20171224161432.27Z", "20171224161432.27Z", 18),
            (b"20171224161432.2Z", "20171224161432.2Z", 17),
            (b"20171224161432Z", "20171224161432Z", 15),
        ];

        let mut write_buffer = BytesMut::with_capacity(1 + tests[0].0.len());

        for (data, input, octet_length) in tests {
            write_buffer.clear();

            write_buffer.put_variable_length_timestamp(&VariableLengthTimestamp {
                inner: Utc
                    .datetime_from_str(input, GENERALIZED_TIME_FORMAT)
                    .unwrap(),
                len: SmallVariableLengthField::from(input.len()),
            });

            assert_eq!(data, &write_buffer[1..].as_ref());
            assert_eq!(octet_length, &write_buffer.as_ref()[0]);
        }
    }
}

#[cfg(test)]
mod fixtures {
    pub static ZERO_LENGTH_VARSTR: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    pub static ONE_BYTE_VARSTR: &[u8] = &[0x01, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    pub static TWO_BYTE_VARSTR: &[u8] = &[0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    /// This buffer is an incorrectly-encoded VarString.
    pub static LENGTH_TOO_HIGH_VARSTR: &[u8] = &[0x07, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
}
