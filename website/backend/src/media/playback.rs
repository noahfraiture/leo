#[derive(Clone, Copy)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

pub fn parse_byte_range(value: &str, total_len: usize) -> Option<ByteRange> {
    if total_len == 0 {
        return None;
    }

    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }

    let (start, end) = spec.split_once('-')?;
    if start.is_empty() {
        return suffix_byte_range(end, total_len);
    }

    let start = start.parse::<usize>().ok()?;
    if start >= total_len {
        return None;
    }

    let end = if end.is_empty() {
        total_len - 1
    } else {
        end.parse::<usize>().ok()?.min(total_len - 1)
    };

    if end < start {
        return None;
    }

    Some(ByteRange { start, end })
}

pub fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some(extension) if extension.eq_ignore_ascii_case("webm") => "video/webm",
        Some(extension) if extension.eq_ignore_ascii_case("mov") => "video/quicktime",
        Some(extension) if extension.eq_ignore_ascii_case("avi") => "video/x-msvideo",
        _ => "video/mp4",
    }
}

fn suffix_byte_range(value: &str, total_len: usize) -> Option<ByteRange> {
    let suffix_len = value.parse::<usize>().ok()?;
    if suffix_len == 0 {
        return None;
    }

    let suffix_len = suffix_len.min(total_len);
    Some(ByteRange {
        start: total_len - suffix_len,
        end: total_len - 1,
    })
}

#[cfg(test)]
mod tests {
    use super::{content_type, parse_byte_range};

    #[test]
    fn parses_open_ended_and_suffix_byte_ranges() {
        let open = parse_byte_range("bytes=2-", 10).expect("open range should parse");
        assert_eq!(open.start, 2);
        assert_eq!(open.end, 9);

        let suffix = parse_byte_range("bytes=-4", 10).expect("suffix range should parse");
        assert_eq!(suffix.start, 6);
        assert_eq!(suffix.end, 9);
    }

    #[test]
    fn rejects_invalid_ranges() {
        assert!(parse_byte_range("bytes=10-12", 10).is_none());
        assert!(parse_byte_range("bytes=4-2", 10).is_none());
        assert!(parse_byte_range("bytes=0-1,3-4", 10).is_none());
        assert!(parse_byte_range("items=0-1", 10).is_none());
    }

    #[test]
    fn maps_common_video_content_types() {
        assert_eq!(content_type("clip.webm"), "video/webm");
        assert_eq!(content_type("clip.mov"), "video/quicktime");
        assert_eq!(content_type("clip.avi"), "video/x-msvideo");
        assert_eq!(content_type("clip.mp4"), "video/mp4");
    }
}
