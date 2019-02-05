use std::io;
use std::io::Write;

struct UtfState {
    wanted: u8,
    seen: u8,
}

struct MaybeString {
    bytes: Vec<u8>,
    inside_utf: Option<UtfState>,
}

struct StringWriter<W> {
    inner: W,
}

#[derive(Copy, Clone, Debug)]
enum ShortArray {
    One([u8; 1]),
    Two([u8; 2]),
    Three([u8; 3]),
    Four([u8; 4]),
}

#[derive(Copy, Clone, Debug)]
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

fn find_chars(data: &[u8]) -> (Vec<Char>, &[u8]) {
    let mut chars = Vec::with_capacity(data.len());

    let mut ptr = data;
    while !ptr.is_empty() {
        match get_char(ptr) {
            Some(c) => {
                chars.push(c);
                ptr = &ptr[c.len()..];
            }
            None => {
                break;
            }
        }
    }

    (chars, ptr)
}

#[derive(Clone, Debug)]
struct Strings<W> {
    output: W,
    buf: Vec<u8>,
    binaries: usize,
}

impl<W> Strings<W> {
    fn new(output: W) -> Strings<W> {
        Strings {
            output,
            buf: Vec::with_capacity(4096),
            binaries: 0,
        }
    }
}

impl<W: Write> Strings<W> {
    fn accept(&mut self, data: &[u8]) -> io::Result<()> {
        let (chars, waste) = find_chars(data);
        for c in chars{
            match c {
                Char::Binary(c) if self.binaries < 2 => {
                    self.binaries += 1;
                    self.buf.push(c);
                }

                Char::Binary(_) => {
                    for _ in 0..self.binaries {
                        assert!(self.buf.pop().is_some());
                    }

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
                    self.binaries = 0;
                }
            }
        }
        Ok(())
    }

    fn finish(mut self) -> io::Result<W> {
        self.output.write_all(&self.buf)?;
        Ok(self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::Strings;

    fn check(expected: &[u8], data: &[u8]) {
        let mut actual = Vec::new();
        let mut state = Strings::new(&mut actual);
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
}
