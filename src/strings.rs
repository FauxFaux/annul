use std::io;
use std::io::Write;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ShortArray {
    One([u8; 1]),
    Two([u8; 2]),
    Three([u8; 3]),
    Four([u8; 4]),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Char {
    Binary(u8),
    Printable(ShortArray),
}

impl Char {
    fn len(&self) -> usize {
        match *self {
            Char::Binary(_) => 1,
            Char::Printable(arr) => arr.len(),
        }
    }
}

impl ShortArray {
    fn len(&self) -> usize {
        match self {
            ShortArray::One(..) => 1,
            ShortArray::Two(..) => 2,
            ShortArray::Three(..) => 3,
            ShortArray::Four(..) => 4,
        }
    }

    fn push_to(&self, v: &mut Vec<u8>) {
        match *self {
            ShortArray::One(a) => v.extend_from_slice(&a),
            ShortArray::Two(a) => v.extend_from_slice(&a),
            ShortArray::Three(a) => v.extend_from_slice(&a),
            ShortArray::Four(a) => v.extend_from_slice(&a),
        }
    }
}

fn get_char(bytes: &[u8]) -> Option<Char> {
    if bytes.is_empty() {
        return None;
    }

    let byte = bytes[0];
    if byte < b' ' && b'\t' != byte && b'\n' != byte && b'\r' != byte {
        return Some(Char::Binary(byte));
    }

    if byte < 0x7f {
        return Some(Char::Printable(ShortArray::One([byte])));
    }

    let wanted = if byte & 0b1110_0000 == 0b1100_0000 {
        2
    } else if byte & 0b1111_0000 == 0b1110_0000 {
        3
    } else if byte & 0b1111_1000 == 0b1111_0000 {
        4
    } else {
        return Some(Char::Binary(byte));
    };

    if bytes.len() < wanted {
        return None;
    }

    for i in 1..wanted {
        if !follower(bytes[i]) {
            return Some(Char::Binary(byte));
        }
    }

    Some(Char::Printable(match wanted {
        2 => ShortArray::Two([bytes[0], bytes[1]]),
        3 => ShortArray::Three([bytes[0], bytes[1], bytes[2]]),
        4 => ShortArray::Four([bytes[0], bytes[1], bytes[2], bytes[3]]),
        _ => unreachable!(),
    }))
}

#[inline]
fn follower(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

struct CharBuf {
    buf: Vec<u8>,
}

impl CharBuf {
    fn push(&mut self, byte: u8) -> Option<Char> {
        self.buf.push(byte);

        let opt = get_char(&self.buf);
        if let Some(c) = opt {
            let _ = self.buf.drain(..c.len());
        }

        opt
    }
}

impl Default for CharBuf {
    fn default() -> Self {
        CharBuf {
            buf: Vec::with_capacity(5),
        }
    }
}

pub struct StringBuf<W> {
    output: W,
    chars: CharBuf,
    buf: Vec<u8>,
    binaries: usize,
}

impl<W: Write> StringBuf<W> {
    pub fn accept(&mut self, buf: &[u8]) -> io::Result<()> {
        for &b in buf {
            self.push(b)?;
        }
        Ok(())
    }

    fn push(&mut self, b: u8) -> io::Result<()> {
        let c = match self.chars.push(b) {
            Some(c) => c,
            None => return Ok(()),
        };

        match c {
            Char::Binary(c) if self.binaries < 2 => {
                self.binaries += 1;
                self.buf.push(c);
            }

            Char::Binary(_) => {
                self.buf.truncate(self.buf.len() - self.binaries);

                if self.buf.len() > 3 {
                    self.output.write_all(&self.buf)?;
                    self.output.write_all(&[0])?;
                }
                self.binaries = 0;
                self.buf.clear()
            }
            Char::Printable(arr) => {
                if self.binaries == self.buf.len() {
                    self.buf.clear();
                }
                arr.push_to(&mut self.buf);
                if self.buf.len() > 255 {
                    self.output.write_all(&self.buf[..250])?;
                    let _ = self.buf.drain(..250);
                }
                self.binaries = 0;
            }
        }

        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.output.write_all(&self.buf)?;
        self.output.write_all(&self.chars.buf)?;
        Ok(self.output)
    }
}

impl<W> StringBuf<W> {
    pub fn new(output: W) -> StringBuf<W> {
        StringBuf {
            chars: CharBuf::default(),
            output,
            buf: Vec::with_capacity(4096),
            binaries: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Char;
    use super::CharBuf;
    use super::ShortArray;
    use super::StringBuf;

    fn check(expected: &[u8], data: &[u8]) {
        let mut actual = Vec::new();
        let mut state = StringBuf::new(&mut actual);
        state.accept(data).expect("only for vec");
        state.finish().expect("only for vec");

        assert_eq!(
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(&actual)
        );
        assert_eq!(expected, actual.as_slice());
    }

    #[test]
    fn strings_all_ascii() {
        check(b"hello", b"hello");
    }

    #[test]
    fn strings_crush_unprintable() {
        check(b"hello\0world", b"hello\0\x01\x02\x03world");
    }

    #[test]
    fn charer() {
        let mut c = CharBuf::default();
        assert_eq!(Some(Char::Printable(ShortArray::One([b'h']))), c.push(b'h'));
        assert_eq!(None, c.push(0b1101_1111));
        assert_eq!(
            Some(Char::Printable(ShortArray::Two([0b1101_1111, 0b1011_1111]))),
            c.push(0b1011_1111)
        );
    }
}
