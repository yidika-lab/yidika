pub const MIN_STATUS: i64 = 100;
pub const MAX_STATUS: i64 = 599;
pub const UNKNOWN_REASON: &str = "Unknown Status Code";

pub const REASONS: &[(i64, &str)] = &[
    (100, "Continue"),
    (101, "Switching Protocols"),
    (102, "Processing"),
    (103, "Early Hints"),
    (200, "OK"),
    (201, "Created"),
    (202, "Accepted"),
    (203, "Non-Authoritative Information"),
    (204, "No Content"),
    (205, "Reset Content"),
    (206, "Partial Content"),
    (207, "Multi-Status"),
    (208, "Already Reported"),
    (226, "IM Used"),
    (300, "Multiple Choices"),
    (301, "Moved Permanently"),
    (302, "Found"),
    (303, "See Other"),
    (304, "Not Modified"),
    (305, "Use Proxy"),
    (307, "Temporary Redirect"),
    (308, "Permanent Redirect"),
    (400, "Bad Request"),
    (401, "Unauthorized"),
    (402, "Payment Required"),
    (403, "Forbidden"),
    (404, "Not Found"),
    (405, "Method Not Allowed"),
    (406, "Not Acceptable"),
    (407, "Proxy Authentication Required"),
    (408, "Request Timeout"),
    (409, "Conflict"),
    (410, "Gone"),
    (411, "Length Required"),
    (412, "Precondition Failed"),
    (413, "Content Too Large"),
    (414, "URI Too Long"),
    (415, "Unsupported Media Type"),
    (416, "Range Not Satisfiable"),
    (417, "Expectation Failed"),
    (418, "I'm a Teapot"),
    (421, "Misdirected Request"),
    (422, "Unprocessable Content"),
    (423, "Locked"),
    (424, "Failed Dependency"),
    (425, "Too Early"),
    (426, "Upgrade Required"),
    (428, "Precondition Required"),
    (429, "Too Many Requests"),
    (431, "Request Header Fields Too Large"),
    (451, "Unavailable For Legal Reasons"),
    (500, "Internal Server Error"),
    (501, "Not Implemented"),
    (502, "Bad Gateway"),
    (503, "Service Unavailable"),
    (504, "Gateway Timeout"),
    (505, "HTTP Version Not Supported"),
    (506, "Variant Also Negotiates"),
    (507, "Insufficient Storage"),
    (508, "Loop Detected"),
    (510, "Not Extended"),
    (511, "Network Authentication Required"),
];

pub fn reason(code: i64) -> &'static str {
    REASONS
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, phrase)| *phrase)
        .unwrap_or(UNKNOWN_REASON)
}

pub fn is_valid(code: i64) -> bool {
    (MIN_STATUS..=MAX_STATUS).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_have_rfc_9110_phrases() {
        assert_eq!(reason(200), "OK");
        assert_eq!(reason(404), "Not Found");
        assert_eq!(reason(422), "Unprocessable Content");
    }

    #[test]
    fn unknown_codes_use_the_fallback() {
        assert_eq!(reason(599), UNKNOWN_REASON);
    }

    #[test]
    fn codes_are_unique_and_in_range() {
        let mut seen = std::collections::HashSet::new();
        for (code, _) in REASONS {
            assert!(is_valid(*code), "{code} out of range");
            assert!(seen.insert(code), "{code} listed twice");
        }
    }
}
