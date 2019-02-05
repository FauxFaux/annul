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
    One(u8),
    Two(u8, u8),
    Three(u8, u8, u8),
    Four(u8, u8, u8, u8),
}

#[derive(Copy, Clone, Debug)]
enum Char {
    Binary(u8),
    Printable(ShortArray),
    Short(usize),
}

impl Char {
    fn len(&self) -> usize {
        match *self {
            Char::Binary(_) => 1,
            Char::Printable(arr) => arr.len(),
            Char::Short(len) => len,
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
            ShortArray::One(a) => v.push(a),
            ShortArray::Two(a, b) => v.extend_from_slice(&[a, b]),
            ShortArray::Three(a, b, c) => v.extend_from_slice(&[a, b, c]),
            ShortArray::Four(a, b, c, d) => v.extend_from_slice(&[a, b, c, d]),
        }
    }
}

fn get_char(bytes: &[u8]) -> Char {
    if bytes.is_empty() {
        return Char::Short(1);
    }

    let byte = bytes[0];
    if byte < b' ' && b'\t' != byte && b'\n' != byte && b'\r' != byte {
        return Char::Binary(byte);
    }

    if byte < 0x7f {
        return Char::Printable(ShortArray::One(byte));
    }

    if byte & 0b1110_0000 == 0b1100_0000 && bytes.len() >= 2 && follower(bytes[1]) {
        return Char::Printable(ShortArray::Two(bytes[0], bytes[1]));
    }

    if byte & 0b1111_0000 == 0b1110_0000
        && bytes.len() >= 3
        && follower(bytes[1])
        && follower(bytes[2])
    {
        return Char::Printable(ShortArray::Three(bytes[0], bytes[1], bytes[2]));
    }

    if byte & 0b1111_1000 == 0b1111_0000
        && bytes.len() >= 4
        && follower(bytes[1])
        && follower(bytes[2])
        && follower(bytes[3])
    {
        return Char::Printable(ShortArray::Four(bytes[0], bytes[1], bytes[2], bytes[3]));
    }

    Char::Binary(byte)
}

#[inline]
fn follower(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn find_chars(data: &[u8]) -> Vec<Char> {
    let mut chars = Vec::with_capacity(data.len());

    let mut ptr = data;
    while !ptr.is_empty() {
        let c = get_char(ptr);
        chars.push(c);
        ptr = &ptr[c.len()..];
    }

    chars
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
        for c in find_chars(data) {
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
                Char::Short(_) => unimplemented!(),
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
