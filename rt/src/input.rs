use std::io::{self, BufRead};

pub const LINE_MAX: usize = 1 << 20;

pub trait LineSource: Send {
    fn next_line(&mut self) -> io::Result<String>;
}

pub struct Stdin;

impl LineSource for Stdin {
    fn next_line(&mut self) -> io::Result<String> {
        read_line(&mut io::stdin().lock(), LINE_MAX)
    }
}

impl LineSource for io::Cursor<Vec<u8>> {
    fn next_line(&mut self) -> io::Result<String> {
        read_line(self, LINE_MAX)
    }
}

pub fn read_line<R: BufRead>(reader: &mut R, limit: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            break;
        }
        let newline = buf.iter().position(|&b| b == b'\n');
        let chunk = &buf[..newline.unwrap_or(buf.len())];
        let room = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = chunk.len() + usize::from(newline.is_some());
        reader.consume(used);
        if newline.is_some() {
            break;
        }
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn lines(input: &[u8], limit: usize, count: usize) -> Vec<String> {
        let mut reader = Cursor::new(input.to_vec());
        (0..count).map(|_| read_line(&mut reader, limit).unwrap()).collect()
    }

    #[test]
    fn the_line_ending_is_removed() {
        assert_eq!(lines(b"bob\nann\r\nend", LINE_MAX, 3), ["bob", "ann", "end"]);
    }

    #[test]
    fn the_end_of_input_reads_as_an_empty_line() {
        assert_eq!(lines(b"", LINE_MAX, 2), ["", ""]);
    }

    #[test]
    fn a_line_over_the_limit_is_cut_and_the_rest_is_skipped() {
        assert_eq!(lines(b"abcdefgh\nxy\n", 3, 2), ["abc", "xy"]);
    }

    #[test]
    fn invalid_utf8_becomes_replacement_characters() {
        assert_eq!(lines(b"a\xffb\n", LINE_MAX, 1), ["a\u{fffd}b"]);
    }

    #[test]
    fn a_line_longer_than_the_reader_buffer_is_read_whole() {
        let long = "x".repeat(20_000);
        let mut reader = io::BufReader::with_capacity(16, Cursor::new(format!("{}\nz\n", long).into_bytes()));
        assert_eq!(read_line(&mut reader, LINE_MAX).unwrap(), long);
        assert_eq!(read_line(&mut reader, LINE_MAX).unwrap(), "z");
    }
}
