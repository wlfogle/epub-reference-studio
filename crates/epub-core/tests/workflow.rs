use epub_reference_core::{align, book, online, workspace::Workspace, Edit, Metadata};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    sync::atomic::AtomicBool,
};
use tempfile::tempdir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const PROSE: &str = "Beyond the amber observatory, patient scholars watched the morning stars fade into a violet horizon. \
Across the eastern valley a river wound between fields of barley and ancient cedar groves. \
The archivist carried a copper lantern, a silver compass, several maps and a leather notebook. \
Nobody could explain why the distant towers glimmered whenever clouds passed over the mountains. \
At dawn their small company crossed the wooden bridge and followed a winding trail toward the city. \
The market gates were painted blue; children waited beside baskets of apples while merchants unpacked their wagons. \
Before midday the travelers stopped beneath an enormous tree and shared bread with the quiet gardener. \
Evening brought clear weather, gentle winds and the scent of flowers from gardens along the western road. \
Later, the captain promised that everyone would return safely before the final bell sounded at the harbor.";

fn epub(path: &Path, body: &str, title: &str, author: &str) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    z.start_file(
        "mimetype",
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
    )
    .unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    for (name, data) in [
        ("META-INF/container.xml", "<container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\"/></rootfiles></container>".to_owned()),
        ("OEBPS/content.opf", format!("<package xmlns=\"http://www.idpf.org/2007/opf\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><metadata><dc:title>{title}</dc:title><dc:creator>{author}</dc:creator><dc:language>en</dc:language><dc:identifier>test:123</dc:identifier></metadata><manifest><item id=\"chapter\" href=\"chapter.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"chapter\"/></spine></package>")),
        ("OEBPS/chapter.xhtml", format!("<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Sample</title></head><body>{body}</body></html>")),
        ("OEBPS/style.css", "p { color: navy; }".into()),
        ("OEBPS/navigation.ncx", "<ncx><navMap/></ncx>".into()),
        ("OEBPS/image.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"/>".into()),
    ] {
        z.start_file(name, deflated).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

fn body(damaged: bool) -> String {
    let sentence = if damaged {
        "The people built a mighty <em>civili</em><b> zation</b> and explored the main street of<span>Varnal.The</span> old sailor took a map from ofme before departing."
    } else {
        "The people built a mighty <em>civilization</em> and explored the main street of<span> Varnal. The</span> old sailor took a map from of me before departing."
    };
    format!(
        "<p>{PROSE}</p><p>{sentence}</p><p>Copper &amp; silver shone in sunlight. {}</p>",
        PROSE.chars().rev().collect::<String>()
    )
}

fn edit(b: &book::Book, before: &str, after: &str) -> Edit {
    let start = b.text.find(before).unwrap();
    Edit {
        id: "fixture".into(),
        start,
        end: start + before.len(),
        before: before.into(),
        after: after.into(),
        context: String::new(),
        location: b.location(start),
        kind: "Reviewed fixture".into(),
    }
}

#[test]
fn reads_metadata_spine_and_inline_text() {
    let d = tempdir().unwrap();
    let path = d.path().join("book.epub");
    epub(&path, &body(true), "The Sample", "A. Writer");
    let b = book::read(&path).unwrap();
    assert_eq!(b.metadata.title, "The Sample");
    assert_eq!(b.metadata.authors, ["A. Writer"]);
    assert!(b.text.contains("civili zation"));
    assert!(b.text.contains("ofVarnal.The"));
    assert!(b.text.contains("Copper & silver"));
    assert!(b.findings.iter().any(|f| f.kind == "Joined words"));
}

#[test]
fn detects_and_exports_real_text_differences_across_inline_tags() {
    let d = tempdir().unwrap();
    let source = d.path().join("damaged.epub");
    let reference = d.path().join("reference.epub");
    epub(&source, &body(true), "The Sample", "A. Writer");
    epub(&reference, &body(false), "The Sample", "Writer, A.");
    let before_hash = book::file_hash(&source).unwrap();
    let b = book::read(&source).unwrap();
    let r = book::read(&reference).unwrap();
    align::identity(&b.metadata, &r.metadata).unwrap();
    let comparison = align::compare(&b, &r.text).unwrap();
    assert!(
        comparison
            .edits
            .iter()
            .any(|e| e.before == "civili zation" && e.after == "civilization"),
        "{:?}",
        comparison.edits
    );
    assert!(comparison
        .edits
        .iter()
        .any(|e| e.before == "ofme" && e.after == "of me"));
    assert!(comparison
        .edits
        .iter()
        .any(|e| e.before.is_empty() && e.after == " "));
    let output = d.path().join("corrected.epub");
    book::export(&b, &source, &output, &comparison.edits).unwrap();
    let repaired = book::read(&output).unwrap();
    assert!(repaired.text.contains("civilization"));
    assert!(repaired.text.contains("of Varnal. The"));
    assert!(repaired.text.contains("of me"));
    assert!(!repaired.text.contains("civili zation"));
    assert_eq!(book::file_hash(&source).unwrap(), before_hash);
    assert_eq!(repaired.metadata.title, b.metadata.title);
    let mut original = ZipArchive::new(File::open(&source).unwrap()).unwrap();
    let mut corrected = ZipArchive::new(File::open(&output).unwrap()).unwrap();
    assert_eq!(corrected.by_index(0).unwrap().name().unwrap(), "mimetype");
    assert_eq!(
        corrected.by_index(0).unwrap().compression(),
        CompressionMethod::Stored
    );
    for name in [
        "OEBPS/style.css",
        "OEBPS/image.svg",
        "OEBPS/content.opf",
        "OEBPS/navigation.ncx",
    ] {
        let mut a = vec![];
        let mut b = vec![];
        original.by_name(name).unwrap().read_to_end(&mut a).unwrap();
        corrected
            .by_name(name)
            .unwrap()
            .read_to_end(&mut b)
            .unwrap();
        assert_eq!(a, b, "Resource changed: {name}");
    }
}

#[test]
fn preserves_entity_and_unicode_mapping_with_multiple_edits() {
    let d = tempdir().unwrap();
    let path = d.path().join("source.epub");
    epub(
        &path,
        "<p>Café &amp; tea of<span>me</span>, then déjà vu and more ofme.</p>",
        "Sample",
        "Writer",
    );
    let b = book::read(&path).unwrap();
    let e1 = edit(&b, "ofme", "of me");
    let mut e2 = e1.clone();
    e2.start = b.text.rfind("ofme").unwrap();
    e2.end = e2.start + 4;
    let output = d.path().join("output.epub");
    book::export(&b, &path, &output, &[e1, e2]).unwrap();
    assert!(book::read(&output)
        .unwrap()
        .text
        .contains("Café & tea of me, then déjà vu and more of me."));
}

#[test]
fn refuses_stale_source_overwrite_and_calibre_output() {
    let d = tempdir().unwrap();
    let path = d.path().join("source.epub");
    epub(&path, &body(true), "Sample", "Writer");
    let b = book::read(&path).unwrap();
    let e = edit(&b, "ofme", "of me");
    assert!(book::export(&b, &path, &path, std::slice::from_ref(&e)).is_err());
    let library = d.path().join("library");
    fs::create_dir(&library).unwrap();
    fs::write(library.join("metadata.db"), "").unwrap();
    assert!(book::export(
        &b,
        &path,
        &library.join("output.epub"),
        std::slice::from_ref(&e)
    )
    .is_err());
    epub(&path, &body(false), "Sample", "Writer");
    assert!(book::export(&b, &path, &d.path().join("output.epub"), &[e]).is_err());
}

#[test]
fn rejects_wrong_work_author_language_and_unrelated_text() {
    let a = Metadata {
        title: "The Sample".into(),
        authors: vec!["Alex Writer".into()],
        language: "en".into(),
        ..Default::default()
    };
    let mut b = a.clone();
    b.authors = vec!["Someone Else".into()];
    assert!(align::identity(&a, &b).is_err());
    b = a.clone();
    b.language = "fr".into();
    assert!(align::identity(&a, &b).is_err());
    b = a.clone();
    b.title = "Different Novel".into();
    assert!(align::identity(&a, &b).is_err());
    let d = tempdir().unwrap();
    let path = d.path().join("book.epub");
    epub(&path, &body(true), "Sample", "Writer");
    assert!(align::compare(&book::read(&path).unwrap(), &"unrelated prose ".repeat(200)).is_err());
}

#[test]
fn unchanged_reference_proposes_no_changes() {
    let d = tempdir().unwrap();
    let p = d.path().join("book.epub");
    epub(&p, &body(false), "Sample", "Writer");
    let b = book::read(&p).unwrap();
    assert!(align::compare(&b, &b.text).unwrap().edits.is_empty());
}

#[test]
fn validates_archive_paths_entities_and_source_urls() {
    for name in ["../outside", "/absolute", "a\\b", "C:/drive", "a/../b"] {
        assert!(book::safe_member(name).is_err());
    }
    assert!(book::xml("<!DOCTYPE x [<!ENTITY e 'bad'>]><x>&e;</x>").is_err());
    for u in [
        "http://gutenberg.org/a",
        "https://127.0.0.1/x",
        "https://gutenberg.org.evil.test/x",
        "https://user:password@gutenberg.org/a",
        "https://gutenberg.org:8443/a",
    ] {
        assert!(online::validate_url(&url::Url::parse(u).unwrap()).is_err());
    }
    assert!(
        online::validate_url(&url::Url::parse("https://www.gutenberg.org/ebooks/11").unwrap())
            .is_ok()
    );
}

#[test]
fn durable_review_and_export_refuse_changed_references() {
    let d = tempdir().unwrap();
    let path = d.path().join("source.epub");
    let reference = d.path().join("reference.epub");
    epub(&path, &body(true), "Sample", "Writer");
    epub(&reference, &body(false), "Sample", "Writer");
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    let j = ws.analyze(&path).unwrap();
    assert_eq!(ws.analyze(&path).unwrap().id, j.id);
    ws.import_reference(&j.id, &reference).unwrap();
    assert!(ws.compare(&j.id, false).is_err());
    let j = ws.compare(&j.id, true).unwrap();
    assert!(j.selected.is_empty());
    let selected = j
        .comparison
        .unwrap()
        .edits
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let j = ws.select(&j.id, selected).unwrap();
    let reloaded = Workspace::new(d.path().join("workspace"))
        .unwrap()
        .job(&j.id)
        .unwrap();
    assert_eq!(j.selected, reloaded.selected);
    assert!(ws.select(&j.id, vec!["stale".into()]).is_err());
    let out = ws.export(&j.id, &d.path().join("exports")).unwrap();
    assert!(out.exists() && out.with_extension("changes.json").exists());
    fs::write(j.reference.unwrap().path, "changed").unwrap();
    assert!(ws.export(&j.id, &d.path().join("exports")).is_err());
}

#[test]
fn library_scan_is_read_only_cached_and_cancellable() {
    let d = tempdir().unwrap();
    let library = d.path().join("library");
    fs::create_dir(&library).unwrap();
    let p = library.join("book.epub");
    epub(&p, &body(true), "Sample", "Writer");
    let hash = book::file_hash(&p).unwrap();
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    let cancel = AtomicBool::new(false);
    let first = ws.scan(&library, &cancel, |_| {}).unwrap();
    assert_eq!(first.books.len(), 1);
    assert!(first.complete);
    assert_eq!(
        ws.scan(&library, &cancel, |_| {}).unwrap().books[0].sha256,
        hash
    );
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(!ws.scan(&library, &cancel, |_| {}).unwrap().complete);
    assert_eq!(book::file_hash(&p).unwrap(), hash);
}

#[test]
fn sigil_reuses_private_copy_and_preserves_saved_manual_edits() {
    use epub_reference_core::sigil::Target;
    let d = tempdir().unwrap();
    let p = d.path().join("original.epub");
    epub(&p, &body(true), "Sample", "Writer");
    let hash = book::file_hash(&p).unwrap();
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    let j = ws.analyze(&p).unwrap();
    let index = j
        .findings
        .iter()
        .position(|f| f.kind == "Joined words")
        .unwrap();
    let handoff = ws.prepare_sigil(&j.id, Target::Finding { index }).unwrap();
    assert_ne!(handoff.path, p);
    assert_eq!(book::file_hash(&handoff.path).unwrap(), hash);
    assert_eq!(handoff.member.as_deref(), Some("OEBPS/chapter.xhtml"));
    assert!(handoff.line.is_some() && !handoff.search_text.is_empty());
    assert!(handoff.notice.contains("not a passage-jump"));
    let repeated = ws.prepare_sigil(&j.id, Target::Book).unwrap();
    assert_eq!(repeated.path, handoff.path);
    epub(&handoff.path, &body(false), "Sample", "Writer");
    let edited_hash = book::file_hash(&handoff.path).unwrap();
    let after = ws.prepare_sigil(&j.id, Target::Finding { index }).unwrap();
    assert_eq!(book::file_hash(&after.path).unwrap(), edited_hash);
    assert!(after.notice.contains("cannot be uniquely located"));
    let review = ws.analyze_sigil(&j.id).unwrap();
    assert_eq!(review.book_path, handoff.path);
    assert_eq!(review.sigil.as_ref().unwrap().path, handoff.path);
    assert_eq!(
        ws.prepare_sigil(&review.id, Target::Book).unwrap().path,
        handoff.path
    );
    assert_eq!(fs::read_dir(ws.path().join("sigil")).unwrap().count(), 1);
    assert_eq!(book::file_hash(&p).unwrap(), hash);
}

#[test]
fn sigil_legacy_findings_and_stale_targets_are_guarded() {
    use epub_reference_core::sigil::Target;
    let d = tempdir().unwrap();
    let p = d.path().join("source.epub");
    epub(&p, &body(true), "Sample", "Writer");
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    let j = ws.analyze(&p).unwrap();
    let path = ws.path().join("jobs").join(format!("{}.json", j.id));
    let mut legacy = serde_json::to_value(&j).unwrap();
    for finding in legacy["findings"].as_array_mut().unwrap() {
        finding.as_object_mut().unwrap().remove("start");
        finding.as_object_mut().unwrap().remove("end");
    }
    legacy.as_object_mut().unwrap().remove("sigil");
    fs::write(path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let target = ws
        .prepare_sigil(&j.id, Target::Finding { index: 0 })
        .unwrap();
    assert!(target.line.is_some());
    assert!(ws
        .prepare_sigil(&j.id, Target::Finding { index: usize::MAX })
        .is_err());
    assert!(ws
        .prepare_sigil(
            &j.id,
            Target::Change {
                id: "invented".into()
            }
        )
        .is_err());
    epub(&p, &body(false), "Sample", "Writer");
    assert!(ws.prepare_sigil(&j.id, Target::Book).is_err());
}

#[test]
fn sigil_refuses_copy_symlinks_and_unregistered_export_paths() {
    use epub_reference_core::sigil::Target;
    let d = tempdir().unwrap();
    let p = d.path().join("source.epub");
    epub(&p, &body(true), "Sample", "Writer");
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    let j = ws.analyze(&p).unwrap();
    assert!(ws
        .prepare_sigil(&j.id, Target::Export { path: p.clone() })
        .is_err());
    let copy = ws.prepare_sigil(&j.id, Target::Book).unwrap().path;
    fs::remove_file(&copy).unwrap();
    std::os::unix::fs::symlink(&p, &copy).unwrap();
    assert!(ws.prepare_sigil(&j.id, Target::Book).is_err());
    assert!(ws.analyze_sigil(&j.id).is_err());
}

#[test]
fn sigil_change_target_maps_to_xhtml_and_does_not_approve() {
    use epub_reference_core::sigil::Target;
    let d = tempdir().unwrap();
    let p = d.path().join("source.epub");
    let r = d.path().join("reference.epub");
    epub(&p, &body(true), "Sample", "Writer");
    epub(&r, &body(false), "Sample", "Writer");
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    let j = ws.analyze(&p).unwrap();
    ws.import_reference(&j.id, &r).unwrap();
    let j = ws.compare(&j.id, true).unwrap();
    let e = &j.comparison.as_ref().unwrap().edits[0];
    let handoff = ws
        .prepare_sigil(&j.id, Target::Change { id: e.id.clone() })
        .unwrap();
    assert!(handoff.line.is_some());
    assert!(ws.job(&j.id).unwrap().selected.is_empty());
}

#[test]
#[ignore = "explicit network smoke test"]
fn live_gutenberg_reference_download() {
    let metadata = Metadata {
        title: "Alice's Adventures in Wonderland".into(),
        authors: vec!["Lewis Carroll".into()],
        language: "en".into(),
        ..Default::default()
    };
    let results = online::search(&metadata, "Alice's Adventures in Wonderland");
    let c = results
        .candidates
        .iter()
        .find(|c| c.identity_match && c.download_url.is_some())
        .unwrap_or_else(|| panic!("No full text: {:?}", results.notices));
    let bytes = online::get(c.download_url.as_ref().unwrap(), book::MAX_ARCHIVE).unwrap();
    let d = tempdir().unwrap();
    let path = d.path().join("reference.epub");
    fs::write(&path, bytes).unwrap();
    let b = book::read(&path).unwrap();
    align::identity(&metadata, &b.metadata).unwrap();
    assert!(b.text.len() > 50_000);
}

#[test]
fn detects_lowercase_fusions_and_corrupt_page_headers() {
    let d = tempdir().unwrap();
    let p = d.path().join("book.epub");
    epub(
        &p,
        "<p>He passed it from ofme. CI1Y OF THE BEAST appeared in the sentence.</p>",
        "Sample",
        "Writer",
    );
    let b = book::read(&p).unwrap();
    for kind in [
        "Function-word fusion",
        "Possible OCR substitution",
        "Possible running header",
    ] {
        assert!(b.findings.iter().any(|f| f.kind == kind), "{kind}");
    }
}

#[test]
fn opds_is_a_full_text_candidate_not_subject_navigation() {
    let meta = Metadata {
        title: "Example Novel".into(),
        authors: vec!["Example Writer".into()],
        ..Default::default()
    };
    let feed = r#"<feed xmlns="http://www.w3.org/2005/Atom">
    <entry><id>https://www.gutenberg.org/ebooks/subjects/search.opds/?query=example</id><title>Subjects</title></entry>
    <entry><id>https://www.gutenberg.org/ebooks/123.opds</id><title>Example Novel</title><content>Example Writer</content></entry></feed>"#;
    let candidates = online::parse_opds(&meta, feed).unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].identity_match);
    assert_eq!(
        candidates[0].download_url.as_deref(),
        Some("https://www.gutenberg.org/ebooks/123.epub3.images")
    );
}

#[test]
fn overlapping_edits_and_arbitrary_session_paths_are_refused() {
    let d = tempdir().unwrap();
    let p = d.path().join("book.epub");
    epub(&p, &body(true), "Sample", "Writer");
    let b = book::read(&p).unwrap();
    let e = edit(&b, "ofme", "of me");
    assert!(book::export(&b, &p, &d.path().join("out.epub"), &[e.clone(), e]).is_err());
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    assert!(ws.job("../other").is_err());
}

#[test]
#[ignore = "explicit installed local model test; fictional excerpts only"]
fn live_local_ollama_assessment_preserves_review_and_uses_cache() {
    use epub_reference_core::ollama;
    let d = tempdir().unwrap();
    let p = d.path().join("book.epub");
    let r = d.path().join("reference.epub");
    epub(&p, &body(true), "Sample", "Writer");
    epub(&r, &body(false), "Sample", "Writer");
    let ws = Workspace::new(d.path().join("workspace")).unwrap();
    let config = ollama::Config::default();
    assert!(ollama::models(&config)
        .unwrap()
        .iter()
        .any(|m| m.name == config.model));
    ws.save_ai_config(config).unwrap();
    let j = ws.analyze(&p).unwrap();
    ws.import_reference(&j.id, &r).unwrap();
    let j = ws.compare(&j.id, true).unwrap();
    let selections = vec![j.comparison.as_ref().unwrap().edits[0].id.clone()];
    ws.select(&j.id, selections.clone()).unwrap();
    let hash = book::file_hash(&p).unwrap();
    let first = ws.assist(&j.id, "differences").unwrap();
    assert!(!first.ai.as_ref().unwrap().items.is_empty());
    assert_eq!(first.selected, selections);
    let second = ws.assist(&j.id, "differences").unwrap();
    assert!(second.ai.unwrap().cache_hit);
    assert_eq!(second.selected, selections);
    assert_eq!(book::file_hash(&p).unwrap(), hash);
}
