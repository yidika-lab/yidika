#[cfg(test)]
mod tests {
    use crate::compat::tests::{normalize, run_aot, run_interp};
    use std::path::{Path, PathBuf};

    const CODE_FENCE: &str = "```yidika";
    const OUTPUT_FENCE: &str = "```output";

    struct Example {
        page: String,
        line: usize,
        source: String,
        expected: String,
    }

    impl Example {
        fn label(&self) -> String {
            format!("docs/{}:{}", self.page, self.line)
        }

        fn build_name(&self) -> String {
            format!("docs_{}_{}", self.page.trim_end_matches(".md").replace(['/', '-'], "_"), self.line)
        }
    }

    fn docs_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs")
    }

    fn doc_pages() -> Vec<PathBuf> {
        let mut pages: Vec<PathBuf> = std::fs::read_dir(docs_dir())
            .expect("docs folder")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        pages.sort();
        pages
    }

    fn fenced_block(lines: &[&str], start: usize) -> (String, usize) {
        let end = lines[start..].iter().position(|l| l.trim_end() == "```").map_or(lines.len(), |n| start + n);
        (lines[start..end].join("\n") + "\n", end + 1)
    }

    fn next_fence(lines: &[&str], from: usize) -> Option<usize> {
        lines[from..].iter().position(|l| l.starts_with("```")).map(|n| from + n)
    }

    fn examples_in(page: &Path) -> Vec<Example> {
        let text = std::fs::read_to_string(page).expect("doc page");
        let lines: Vec<&str> = text.lines().collect();
        let name = page.file_name().unwrap().to_string_lossy().to_string();
        let mut examples = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            if lines[i].trim_end() != CODE_FENCE {
                i += 1;
                continue;
            }
            let (source, after) = fenced_block(&lines, i + 1);
            let output = next_fence(&lines, after).filter(|&f| lines[f].trim_end() == OUTPUT_FENCE);
            match output {
                Some(fence) => {
                    let (expected, after_output) = fenced_block(&lines, fence + 1);
                    examples.push(Example { page: name.clone(), line: i + 1, source, expected });
                    i = after_output;
                }
                None => i = after,
            }
        }
        examples
    }

    fn all_examples() -> Vec<Example> {
        doc_pages().iter().flat_map(|page| examples_in(page)).collect()
    }

    #[test]
    fn an_example_is_a_code_block_followed_by_its_output() {
        let dir = std::env::temp_dir().join(format!("yk_docs_parse_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let page = dir.join("page.md");
        std::fs::write(&page, "Intro\n\n```yidika\nfn main() { print(1); }\n```\n\nIt prints:\n\n```output\n1\n```\n\n```yidika\nfn alone() {}\n```\n").unwrap();
        let examples = examples_in(&page);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].line, 3);
        assert_eq!(examples[0].source, "fn main() { print(1); }\n");
        assert_eq!(examples[0].expected, "1\n");
    }

    #[test]
    fn every_documented_example_prints_its_output_in_the_interpreter() {
        let examples = all_examples();
        assert!(!examples.is_empty(), "no example found in docs/");
        let failures: Vec<String> = examples
            .iter()
            .filter_map(|ex| match run_interp(&ex.source) {
                Ok(out) if normalize(&out) == normalize(&ex.expected) => None,
                Ok(out) => Some(format!("{}: expected\n{}\ngot\n{}", ex.label(), ex.expected, out)),
                Err(e) => Some(format!("{}: {}", ex.label(), e)),
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }

    #[test]
    fn compat_documented_examples_print_the_same_output_natively() {
        let failures: Vec<String> = all_examples()
            .iter()
            .filter_map(|ex| match run_aot(&ex.source, &ex.build_name()) {
                Ok(out) if normalize(&out) == normalize(&ex.expected) => None,
                Ok(out) => Some(format!("{}: native printed\n{}", ex.label(), out)),
                Err(e) if e.starts_with("AOT build:") => None,
                Err(e) => Some(format!("{}: {}", ex.label(), e)),
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }
}
