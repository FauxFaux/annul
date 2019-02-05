use std::io::Read;

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

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Happy,
    Suspicious(u8),
    Miffed(u8, u8),
    Angry,
    Utf2(Vec<u8>),
    Utf3(Vec<u8>),
    Utf4(Vec<u8>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Classification {
    Binary,
    Ascii,
    Utf2,
    Utf3,
    Utf4,
    UtfFollower,
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

fn classify(byte: u8) -> Classification {
    if byte < b' ' && b'\t' != byte && b'\n' != byte && b'\r' != byte {
        return Classification::Binary;
    }

    if byte < 0x7f {
        return Classification::Ascii;
    }

    if byte & 0b1100_0000 == 0b1000_0000 {
        return Classification::UtfFollower;
    }

    if byte & 0b1110_0000 == 0b1100_0000 {
        return Classification::Utf2;
    }

    if byte & 0b1111_0000 == 0b1110_0000 {
        return Classification::Utf3;
    }

    if byte & 0b1111_1000 == 0b1111_0000 {
        return Classification::Utf4;
    }

    Classification::Binary
}

fn find_chars(data: &[u8]) -> Vec<Char> {
    let mut chars = Vec::with_capacity(data.len());

    let mut ptr = data;
    while !ptr.is_empty() {
        let c = get_char(ptr);
        match c {
            Char::Short(missing) => {
                // TODO
                assert_eq!(0, missing);
                break;
            },
            other => chars.push(other),
        }
        ptr = &ptr[c.len()..];
    }

    chars
}

fn strings(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut buf = Vec::with_capacity(12);
    let mut binaries = 0;
    for c in find_chars(data) {
        match c {
            Char::Binary(c) if binaries < 2 => {
                binaries += 1;
                buf.push(c);
            }

            Char::Binary(_) => {
                for _ in 0..binaries {
                    assert!(buf.pop().is_some());
                }

                if buf.len() > 3 {
                    out.extend_from_slice(&buf);
                    out.push(0);
                }
                binaries = 0;
                buf.clear()
            },
            Char::Printable(arr) => {
                if binaries == buf.len() {
                    buf.clear();
                }
                arr.push_to(&mut buf);
                binaries = 0;
            },
            Char::Short(_) => unimplemented!(),
        }
    }
    out.extend_from_slice(&buf);
    out
}

#[cfg(never)]
fn satrings(data: &[u8]) -> Vec<u8> {
    let mut ret = Vec::with_capacity(data.len());

    let mut state = State::Happy;

    for &b in data {
        let classification = classify(b);
        state = match state {
            State::Happy => match classification {
                Classification::Ascii => {
                    ret.push(b);
                    State::Happy
                }
                Classification::Binary | Classification::UtfFollower => State::Suspicious(b),
                Classification::Utf2 => State::Utf2(vec![b]),
                Classification::Utf3 => State::Utf3(vec![b]),
                Classification::Utf4 => State::Utf4(vec![b]),
            },
        }
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::strings;

    fn check(expected: &[u8], data: &[u8]) {
        let actual = strings(data);

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
