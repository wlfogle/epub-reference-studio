use crate::{Finding, Metadata};
use anyhow::{bail, ensure, Context, Result};
use percent_encoding::percent_decode_str;
use regex::Regex;
use roxmltree::{Document, Node, ParsingOptions};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    ops::Range,
    path::{Component, Path, PathBuf},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

pub const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
pub const MAX_EXPANDED: u64 = 256 * 1024 * 1024;
pub const MAX_ENTRY: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Segment {
    pub member: String,
    pub raw: Range<usize>,
    pub text: Range<usize>,
    pub decoded: String,
}

#[derive(Debug)]
pub struct Book {
    pub metadata: Metadata,
    pub sha256: String,
    pub text: String,
    pub segments: Vec<Segment>,
    pub documents: HashMap<String, String>,
    pub findings: Vec<Finding>,
}

pub fn safe_member(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && !name.contains(['\\', '\0', ':']),
        "Unsafe ZIP path"
    );
    ensure!(
        Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "Absolute or traversing ZIP path: {name}"
    );
    Ok(())
}

fn resolve(base: &str, href: &str) -> Result<String> {
    let href = percent_decode_str(href.split('#').next().unwrap_or("")).decode_utf8()?;
    ensure!(
        !href.contains(['\\', ':']) && !href.starts_with('/'),
        "Unsafe manifest URI"
    );
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    parts.pop();
    for part in href.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                ensure!(parts.pop().is_some(), "Manifest escapes archive");
            }
            p => parts.push(p),
        }
    }
    let path = parts.join("/");
    safe_member(&path)?;
    Ok(path)
}

pub fn xml(text: &str) -> Result<Document<'_>> {
    ensure!(
        !text.contains("<!ENTITY"),
        "Custom XML entities are not supported"
    );
    Ok(Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: true,
            nodes_limit: 500_000,
            ..Default::default()
        },
    )?)
}

fn entry(z: &mut ZipArchive<File>, name: &str) -> Result<String> {
    let f = z.by_name(name)?;
    ensure!(f.size() <= MAX_ENTRY, "Oversized XML entry");
    let mut data = String::new();
    f.take(MAX_ENTRY + 1)
        .read_to_string(&mut data)
        .context("EPUB XML must be UTF-8")?;
    ensure!(
        data.len() as u64 <= MAX_ENTRY,
        "Expanded XML entry too large"
    );
    Ok(data)
}

pub fn read(path: &Path) -> Result<Book> {
    ensure!(
        path.is_file() && path.metadata()?.len() <= MAX_ARCHIVE,
        "EPUB missing or too large"
    );
    let sha256 = file_hash(path)?;
    let mut z = ZipArchive::new(File::open(path)?)?;
    ensure!(z.len() <= 20_000, "Too many ZIP entries");
    let mut names = HashSet::new();
    let mut expanded = 0u64;
    for i in 0..z.len() {
        let f = z.by_index(i)?;
        let name = f.name()?.into_owned();
        safe_member(name.trim_end_matches('/'))?;
        ensure!(names.insert(name), "Duplicate ZIP entry");
        expanded = expanded
            .checked_add(f.size())
            .context("ZIP size overflow")?;
        ensure!(
            f.size() <= MAX_ENTRY && expanded <= MAX_EXPANDED,
            "EPUB expansion limit exceeded"
        );
        ensure!(
            f.unix_mode().unwrap_or(0) & 0o170000 != 0o120000,
            "ZIP symlink refused"
        );
    }
    ensure!(
        entry(&mut z, "mimetype")? == "application/epub+zip",
        "Not an EPUB"
    );
    if names.contains("META-INF/encryption.xml") {
        let encryption = entry(&mut z, "META-INF/encryption.xml")?;
        let d = xml(&encryption)?;
        for cipher in d
            .descendants()
            .filter(|n| n.has_tag_name("CipherReference"))
        {
            let uri = cipher.attribute("URI").unwrap_or("");
            ensure!(
                uri.to_lowercase().ends_with(".otf") || uri.to_lowercase().ends_with(".ttf"),
                "Encrypted ebook text is not supported"
            );
        }
    }
    let container = entry(&mut z, "META-INF/container.xml")?;
    let c = xml(&container)?;
    let opf = c
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .context("EPUB container has no package path")?
        .to_owned();
    safe_member(&opf)?;
    let package = entry(&mut z, &opf)?;
    let d = xml(&package)?;
    let texts = |tag: &str| -> Vec<String> {
        d.descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == tag)
            .filter_map(|n| n.text())
            .map(|s| s.trim().to_owned())
            .collect()
    };
    let metadata = Metadata {
        title: texts("title").first().cloned().unwrap_or_default(),
        authors: texts("creator"),
        language: texts("language").first().cloned().unwrap_or_default(),
        identifiers: texts("identifier"),
    };
    let manifest: HashMap<_, _> = d
        .descendants()
        .filter(|n| n.has_tag_name("item"))
        .filter_map(|n| {
            Some((
                n.attribute("id")?.to_owned(),
                n.attribute("href")?.to_owned(),
            ))
        })
        .collect();
    let mut result = Book {
        metadata,
        sha256,
        text: String::new(),
        segments: vec![],
        documents: HashMap::new(),
        findings: vec![],
    };
    let mut spine = HashSet::new();
    for id in d
        .descendants()
        .filter(|n| n.has_tag_name("itemref"))
        .filter_map(|n| n.attribute("idref"))
    {
        let href = manifest
            .get(id)
            .context("Spine item absent from manifest")?;
        let member = resolve(&opf, href)?;
        ensure!(spine.insert(member.clone()), "Repeated spine member");
        let raw = entry(&mut z, &member)?;
        result.add_document(&member, raw)?;
    }
    ensure!(
        !result.text.trim().is_empty(),
        "No readable text in EPUB spine"
    );
    ensure!(
        result.text.len() <= 16 * 1024 * 1024,
        "Book text exceeds analysis limit"
    );
    result.findings = detect(&result);
    Ok(result)
}

fn block(node: Node<'_, '_>) -> Option<roxmltree::NodeId> {
    node.ancestors()
        .find(|n| {
            n.is_element()
                && matches!(
                    n.tag_name().name(),
                    "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "blockquote"
                )
        })
        .map(|n| n.id())
}

impl Book {
    fn add_document(&mut self, member: &str, raw: String) -> Result<()> {
        let d = xml(&raw).with_context(|| format!("Invalid XHTML in {member}"))?;
        let body = d
            .descendants()
            .find(|n| n.has_tag_name("body"))
            .context("XHTML has no body")?;
        let mut last_block = None;
        self.text.push_str("\n\n");
        for n in body.descendants().filter(|n| n.is_text()) {
            if n.ancestors().any(|a| {
                a.is_element() && matches!(a.tag_name().name(), "script" | "style" | "noscript")
            }) {
                continue;
            }
            let text = n.text().unwrap_or("");
            let current = block(n);
            if current != last_block {
                self.text.push_str("\n\n");
                last_block = current;
            }
            let start = self.text.len();
            self.text.push_str(text);
            self.segments.push(Segment {
                member: member.into(),
                raw: n.range(),
                text: start..self.text.len(),
                decoded: text.into(),
            });
        }
        self.documents.insert(member.into(), raw);
        Ok(())
    }

    pub fn location(&self, offset: usize) -> String {
        self.segments
            .iter()
            .find(|s| s.text.start <= offset && offset <= s.text.end)
            .map(|s| s.member.clone())
            .unwrap_or_default()
    }
}

pub fn file_hash(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut f = File::open(path)?;
    let mut buffer = [0u8; 65536];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{hash:x}", hash = hash.finalize()))
}

pub fn context(text: &str, start: usize, end: usize) -> String {
    let mut a = start.saturating_sub(65);
    let mut b = (end + 65).min(text.len());
    while !text.is_char_boundary(a) {
        a += 1;
    }
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    text[a..b].split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn detect(book: &Book) -> Vec<Finding> {
    let mut findings = vec![];
    let rules = [
        ("Joined words", r"\b(?:of|the|and|in|to|from|with|at|by|for)[A-Z][a-z]{2,}\b",
         "A common function word appears joined to a capitalized word; compare with a reference."),
        ("Missing sentence space", r"\p{Ll}[.!?][\x22\x27’”]?\p{Lu}",
         "Sentence punctuation has no following space; abbreviations may be intentional."),
        ("Function-word fusion", r"\b(?:of|from|with|for|at|by)(?:me|you|him|her|us|them|the|an|my|your|his|their|our)\b",
         "Two function words may have lost their separating space; verify against the reference."),
        ("Possible running header", r"\b[\p{Lu}01]{2,}(?:[ \t]+[\p{Lu}01]{2,}){2,}\b",
         "An all-capitals phrase may be an embedded page header or an intentional heading; inspect its context."),
        ("Possible OCR substitution", r"\b(?:\p{L}+[01]\p{L}+|[01]\p{L}{2,})\b",
         "Digit inside a word may be an OCR substitution; preserve intentional names/numbers."),
        ("Broken word", r"\b\p{L}{2,12}[-¬]\s+\p{Ll}{2,}\b",
         "A line-end hyphen may have survived conversion; genuine hyphenated words need review."),
    ];
    for (kind, pattern, explanation) in rules {
        let re = Regex::new(pattern).expect("constant expression");
        for m in re.find_iter(&book.text).take(400) {
            findings.push(Finding {
                kind: kind.into(),
                location: book.location(m.start()),
                context: context(&book.text, m.start(), m.end()),
                explanation: explanation.into(),
            });
        }
    }
    let words = Regex::new(r"\p{L}+").expect("constant expression");
    let mut frequencies: HashMap<String, usize> = HashMap::new();
    for m in words.find_iter(&book.text) {
        *frequencies.entry(m.as_str().to_lowercase()).or_default() += 1;
    }
    let split = Regex::new(r"\b(\p{L}{2,})[ \t\r\n]+(\p{Ll}{2,})\b").expect("constant expression");
    for c in split.captures_iter(&book.text) {
        let joined = format!("{}{}", &c[1], &c[2]).to_lowercase();
        let left = c[1].to_lowercase();
        let right = c[2].to_lowercase();
        if frequencies.get(&joined).copied().unwrap_or(0) >= 2
            && frequencies.get(&left).copied().unwrap_or(0) <= 2
            && frequencies.get(&right).copied().unwrap_or(0) <= 2
        {
            let m = c.get(0).expect("matched capture");
            findings.push(Finding {
                kind: "Possible split word".into(),
                location: book.location(m.start()),
                context: context(&book.text, m.start(), m.end()),
                explanation: format!("“{joined}” occurs elsewhere intact; verify before joining."),
            });
        }
        if findings.len() >= 2000 {
            break;
        }
    }
    findings
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn export(book: &Book, source: &Path, output: &Path, edits: &[crate::Edit]) -> Result<()> {
    ensure!(!edits.is_empty(), "Select at least one reviewed correction");
    ensure!(
        file_hash(source)? == book.sha256,
        "Original EPUB changed; analyze again"
    );
    ensure!(
        !output.exists(),
        "Output already exists; refusing overwrite"
    );
    let parent = output.parent().context("Output has no directory")?;
    std::fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    ensure!(
        !parent.ancestors().any(|p| p.join("metadata.db").exists()),
        "Export outside the Calibre library to preserve managed originals"
    );
    let mut ordered = edits.to_vec();
    ordered.sort_by_key(|e| (e.start, e.end));
    let mut previous = None;
    for e in &ordered {
        ensure!(
            e.start <= e.end && book.text.get(e.start..e.end) == Some(&e.before),
            "Stale or invalid correction"
        );
        if let Some((start, end)) = previous {
            ensure!(
                e.start >= end && !(e.start == start && (start == end || e.start == e.end)),
                "Overlapping corrections"
            );
        }
        previous = Some((e.start, e.end));
    }
    let mut replacements: HashMap<(String, usize, usize), String> = HashMap::new();
    for e in ordered.iter().rev() {
        let nodes: Vec<_> = book
            .segments
            .iter()
            .filter(|s| {
                if e.start == e.end {
                    s.text.start <= e.start && e.start <= s.text.end
                } else {
                    s.text.end > e.start && s.text.start < e.end
                }
            })
            .collect();
        ensure!(
            !nodes.is_empty(),
            "Correction does not map to an XHTML text node"
        );
        for (i, segment) in nodes.iter().enumerate() {
            if e.start == e.end && i > 0 {
                break;
            }
            let key = (segment.member.clone(), segment.raw.start, segment.raw.end);
            let value = replacements
                .entry(key)
                .or_insert_with(|| segment.decoded.clone());
            let a = e
                .start
                .saturating_sub(segment.text.start)
                .min(segment.decoded.len());
            let b = e
                .end
                .saturating_sub(segment.text.start)
                .min(segment.decoded.len());
            ensure!(
                value.is_char_boundary(a) && value.is_char_boundary(b),
                "Unicode boundary mismatch"
            );
            value.replace_range(a..b, if i == 0 { &e.after } else { "" });
        }
    }
    let mut documents = book.documents.clone();
    let mut raw_patches: Vec<_> = replacements.into_iter().collect();
    raw_patches.sort_by(|a, b| b.0.cmp(&a.0));
    for ((member, start, end), text) in raw_patches {
        let raw = documents.get_mut(&member).context("Missing XHTML member")?;
        raw.replace_range(start..end, &escape(&text));
    }
    for doc in documents.values() {
        xml(doc).context("Corrected XHTML failed validation")?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
    {
        let mut writer = ZipWriter::new(temp.as_file_mut());
        writer.start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )?;
        writer.write_all(b"application/epub+zip")?;
        let mut archive = ZipArchive::new(File::open(source)?)?;
        for i in 0..archive.len() {
            let f = archive.by_index(i)?;
            let name = f.name()?.into_owned();
            if name == "mimetype" {
                continue;
            }
            if let Some(raw) = documents.get(&name) {
                writer.start_file(
                    &name,
                    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
                )?;
                writer.write_all(raw.as_bytes())?;
            } else {
                writer.raw_copy_file(f)?;
            }
        }
        writer.finish()?;
    }
    temp.as_file_mut().sync_all()?;
    let checked = read(temp.path()).context("Export verification failed")?;
    let mut expected = book.text.clone();
    for e in ordered.iter().rev() {
        expected.replace_range(e.start..e.end, &e.after);
    }
    let compact = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(
        compact(&checked.text) == compact(&expected),
        "Exported visible text does not match the exact reviewed corrections"
    );
    ensure!(
        file_hash(source)? == book.sha256,
        "Source changed during export"
    );
    temp.persist_noclobber(output)
        .map_err(|e| anyhow::anyhow!("Cannot publish output: {e}"))?;
    Ok(())
}

pub fn normalized_path(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.is_file() {
        bail!("Not a regular file");
    }
    Ok(path)
}
