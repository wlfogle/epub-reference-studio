use crate::{align, book, digest, now, ollama, online, Comparison, Finding, Metadata};
use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub path: PathBuf,
    pub sha256: String,
    pub metadata: Metadata,
    pub findings: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    pub root: PathBuf,
    pub books: Vec<Row>,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub path: PathBuf,
    pub sha256: String,
    pub metadata: Metadata,
    pub source: String,
    pub acquired: u64,
    pub plain_text: bool,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub book_path: PathBuf,
    pub input_sha256: String,
    pub metadata: Metadata,
    pub findings: Vec<Finding>,
    pub reference: Option<Reference>,
    pub search: Option<online::Search>,
    pub comparison: Option<Comparison>,
    pub selected: Vec<String>,
    pub exports: Vec<PathBuf>,
    pub updated: u64,
    #[serde(default)]
    pub ai: Option<ollama::Assessment>,
}

#[derive(Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        ensure!(
            !root.ancestors().any(|p| p.join("metadata.db").exists()),
            "Application workspace must be outside Calibre libraries"
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        for d in ["jobs", "references", "search", "ai"] {
            fs::create_dir_all(root.join(d))?;
            fs::set_permissions(root.join(d), fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { root })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    fn lock(&self) -> Result<File> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("workspace.lock"))?;
        f.try_lock().map_err(|e| {
            anyhow::anyhow!("Another application operation holds this workspace: {e}")
        })?;
        Ok(f)
    }

    fn json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let mut tmp = tempfile::NamedTempFile::new_in(path.parent().context("No parent")?)?;
        serde_json::to_writer(tmp.as_file_mut(), value)?;
        tmp.write_all(b"\n")?;
        tmp.as_file_mut().sync_all()?;
        tmp.persist(path)
            .map_err(|e| anyhow::anyhow!("Cannot save session: {e}"))?;
        Ok(())
    }

    fn load<T: DeserializeOwned>(&self, path: &Path) -> Result<T> {
        ensure!(
            path.metadata()?.len() <= 64 * 1024 * 1024,
            "Session file exceeds size limit"
        );
        Ok(serde_json::from_reader(File::open(path)?)?)
    }

    fn job_path(&self, id: &str) -> Result<PathBuf> {
        ensure!(
            id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()),
            "Invalid job ID"
        );
        Ok(self.root.join("jobs").join(format!("{id}.json")))
    }

    pub fn job(&self, id: &str) -> Result<Job> {
        self.load(&self.job_path(id)?)
    }

    fn save(&self, job: &mut Job) -> Result<()> {
        job.updated = now();
        self.json(&self.job_path(&job.id)?, job)
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        let mut jobs = vec![];
        for f in fs::read_dir(self.root.join("jobs"))?.take(5000) {
            let p = f?.path();
            if p.extension().is_some_and(|e| e == "json") {
                jobs.push(self.load::<Job>(&p)?);
            }
        }
        jobs.sort_by_key(|j| std::cmp::Reverse(j.updated));
        Ok(jobs)
    }

    pub fn library(&self) -> Result<Library> {
        let p = self.root.join("library.json");
        if p.exists() {
            self.load(&p)
        } else {
            Ok(Library::default())
        }
    }

    pub fn scan(
        &self,
        root: &Path,
        cancel: &AtomicBool,
        progress: impl Fn(usize),
    ) -> Result<Library> {
        let _guard = self.lock()?;
        let root = root.canonicalize()?;
        ensure!(root.is_dir(), "Select a library directory");
        let previous = self.library()?;
        let old: std::collections::HashMap<_, _> = previous
            .books
            .into_iter()
            .filter(|r| r.error.is_none())
            .map(|r| (r.path.clone(), r))
            .collect();
        let mut library = Library {
            root: root.clone(),
            books: vec![],
            complete: true,
        };
        for entry in walkdir::WalkDir::new(&root)
            .follow_links(false)
            .max_depth(32)
        {
            if cancel.load(Ordering::Relaxed) {
                library.complete = false;
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    library.books.push(Row {
                        path: e.path().unwrap_or(&root).into(),
                        sha256: String::new(),
                        metadata: Metadata::default(),
                        findings: 0,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            };
            if !entry.file_type().is_file()
                || !entry
                    .path()
                    .extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("epub"))
            {
                continue;
            }
            ensure!(
                library.books.len() < 50_000,
                "Library limit is 50,000 EPUBs per scan"
            );
            let parsed = book::file_hash(entry.path()).and_then(|hash| {
                if let Some(row) = old.get(entry.path()).filter(|r| r.sha256 == hash) {
                    Ok(row.clone())
                } else {
                    let b = book::read(entry.path())?;
                    Ok(Row {
                        path: entry.path().into(),
                        sha256: b.sha256,
                        metadata: b.metadata,
                        findings: b.findings.len(),
                        error: None,
                    })
                }
            });
            library.books.push(parsed.unwrap_or_else(|e| Row {
                path: entry.path().into(),
                sha256: String::new(),
                metadata: Metadata::default(),
                findings: 0,
                error: Some(format!("{e:#}")),
            }));
            progress(library.books.len());
            if library.books.len().is_multiple_of(20) {
                let mut partial = library.clone();
                partial.complete = false;
                self.json(&self.root.join("library.json"), &partial)?;
            }
        }
        library.books.sort_by_key(|r| std::cmp::Reverse(r.findings));
        self.json(&self.root.join("library.json"), &library)?;
        Ok(library)
    }

    pub fn analyze(&self, path: &Path) -> Result<Job> {
        let _guard = self.lock()?;
        let path = book::normalized_path(path)?;
        let b = book::read(&path)?;
        if let Some(j) = self
            .jobs()?
            .into_iter()
            .find(|j| j.book_path == path && j.input_sha256 == b.sha256)
        {
            return Ok(j);
        }
        let mut job = Job {
            id: uuid::Uuid::new_v4().simple().to_string(),
            book_path: path,
            input_sha256: b.sha256,
            metadata: b.metadata,
            findings: b.findings,
            reference: None,
            search: None,
            comparison: None,
            selected: vec![],
            exports: vec![],
            updated: now(),
            ai: None,
        };
        self.save(&mut job)?;
        Ok(job)
    }

    pub fn search(&self, id: &str, query: &str, refresh: bool) -> Result<Job> {
        let _guard = self.lock()?;
        ensure!(query.len() <= 500, "Search query too long");
        let mut job = self.job(id)?;
        let key = digest(
            format!(
                "{}:{}:{query}",
                job.metadata.title,
                job.metadata.authors.join("|")
            )
            .as_bytes(),
        );
        let cache = self.root.join("search").join(format!("{key}.json"));
        let cached: Option<(u64, online::Search)> = if cache.exists() {
            self.load(&cache).ok()
        } else {
            None
        };
        let search =
            match cached.filter(|(time, _)| !refresh && now().saturating_sub(*time) < 86400) {
                Some((_, s)) => s,
                None => {
                    let s = online::search(&job.metadata, query);
                    self.json(&cache, &(now(), &s))?;
                    s
                }
            };
        job.search = Some(search);
        self.save(&mut job)?;
        Ok(job)
    }

    fn attach(
        &self,
        job: &mut Job,
        path: &Path,
        source: String,
        hint: Option<Metadata>,
    ) -> Result<()> {
        let path = book::normalized_path(path)?;
        ensure!(
            path.metadata()?.len() <= book::MAX_ARCHIVE,
            "Reference file too large"
        );
        let plain_text = path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("txt"));
        let metadata = if plain_text {
            ensure!(
                path.metadata()?.len() <= 16 * 1024 * 1024,
                "Reference text too large"
            );
            let text = fs::read_to_string(&path).context("Reference must be UTF-8 text")?;
            ensure!(
                !text.contains('\0') && text.trim().len() >= 80,
                "Invalid reference text"
            );
            hint.unwrap_or_default()
        } else {
            book::read(&path)?.metadata
        };
        // EPUB identities are checked now; plain text needs explicit confirmation later.
        if !plain_text {
            align::identity(&job.metadata, &metadata)?;
        }
        let sha256 = book::file_hash(&path)?;
        let destination = self.root.join("references").join(format!(
            "{}.{}",
            sha256,
            if plain_text { "txt" } else { "epub" }
        ));
        if destination.exists() {
            ensure!(
                book::file_hash(&destination)? == sha256,
                "Cached reference hash mismatch"
            );
        } else {
            let mut temp = tempfile::NamedTempFile::new_in(self.root.join("references"))?;
            std::io::copy(&mut File::open(&path)?, temp.as_file_mut())?;
            temp.as_file_mut().sync_all()?;
            ensure!(
                book::file_hash(temp.path())? == sha256,
                "Reference changed during import"
            );
            temp.persist_noclobber(&destination)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        job.reference = Some(Reference {
            path: destination,
            sha256,
            metadata,
            source,
            acquired: now(),
            plain_text,
            confirmed: false,
        });
        job.comparison = None;
        job.ai = None;
        job.selected.clear();
        Ok(())
    }

    pub fn import_reference(&self, id: &str, path: &Path) -> Result<Job> {
        let _guard = self.lock()?;
        let mut job = self.job(id)?;
        self.attach(
            &mut job,
            path,
            format!("Local user-selected file: {}", path.display()),
            None,
        )?;
        self.save(&mut job)?;
        Ok(job)
    }

    pub fn download(&self, id: &str, candidate: &str) -> Result<Job> {
        let _guard = self.lock()?;
        let mut job = self.job(id)?;
        let c = job
            .search
            .as_ref()
            .and_then(|s| s.candidates.iter().find(|c| c.id == candidate))
            .context("Search again to select a current candidate")?
            .clone();
        ensure!(
            c.identity_match,
            "Candidate title/author/translation mismatch"
        );
        let url = c
            .download_url
            .context("This catalogue entry supplies no downloadable reference")?;
        let bytes = online::get(&url, book::MAX_ARCHIVE)?;
        let plain = c.format.starts_with("text/plain");
        let temp = tempfile::Builder::new()
            .suffix(if plain { ".txt" } else { ".epub" })
            .tempfile_in(self.root.join("references"))?;
        fs::write(temp.path(), bytes)?;
        self.attach(
            &mut job,
            temp.path(),
            format!("{} — {}", c.provider, url),
            Some(c.metadata),
        )?;
        self.save(&mut job)?;
        Ok(job)
    }

    pub fn compare(&self, id: &str, confirmed: bool) -> Result<Job> {
        ensure!(
            confirmed,
            "Confirm the same work, edition, translation and reference rights first"
        );
        let _guard = self.lock()?;
        let mut job = self.job(id)?;
        let b = book::read(&job.book_path)?;
        ensure!(
            b.sha256 == job.input_sha256,
            "Original changed; analyze again"
        );
        let r = job
            .reference
            .as_mut()
            .context("Acquire a full-text reference first")?;
        ensure!(
            book::file_hash(&r.path)? == r.sha256,
            "Reference changed; import again"
        );
        let text = if r.plain_text {
            fs::read_to_string(&r.path)?
        } else {
            let reference = book::read(&r.path)?;
            align::identity(&b.metadata, &reference.metadata)?;
            reference.text
        };
        job.comparison = Some(align::compare(&b, &text)?);
        job.ai = None;
        r.confirmed = true;
        job.selected.clear();
        self.save(&mut job)?;
        Ok(job)
    }

    pub fn ai_config(&self) -> Result<ollama::Config> {
        let path = self.root.join("ollama.json");
        if path.exists() {
            self.load(&path)
        } else {
            Ok(ollama::Config::default())
        }
    }

    pub fn save_ai_config(&self, config: ollama::Config) -> Result<ollama::Config> {
        let _guard = self.lock()?;
        ollama::model_digest(&config)?;
        self.json(&self.root.join("ollama.json"), &config)?;
        Ok(config)
    }

    pub fn assist(&self, id: &str, mode: &str) -> Result<Job> {
        let _guard = self.lock()?;
        let mut job = self.job(id)?;
        ensure!(
            book::file_hash(&job.book_path)? == job.input_sha256,
            "Original changed; analyze before requesting advice"
        );
        let inputs: Vec<ollama::Input> = match mode {
            "differences" => {
                let reference = job.reference.as_ref().context("No reference")?;
                ensure!(
                    book::file_hash(&reference.path)? == reference.sha256,
                    "Reference changed; compare again"
                );
                job.comparison
                    .as_ref()
                    .context("Compare a reference first")?
                    .edits
                    .iter()
                    .filter(|e| job.selected.is_empty() || job.selected.contains(&e.id))
                    .take(8)
                    .map(|e| ollama::Input {
                        id: e.id.clone(),
                        suspect: e.before.clone(),
                        reference: Some(e.after.clone()),
                        context: e.context.clone(),
                    })
                    .collect()
            }
            "findings" => job
                .findings
                .iter()
                .take(8)
                .map(|f| ollama::Input {
                    id: digest(format!("{}:{}:{}", f.kind, f.location, f.context).as_bytes()),
                    suspect: f.context.clone(),
                    reference: None,
                    context: f.explanation.clone(),
                })
                .collect(),
            _ => anyhow::bail!("Unknown assessment mode"),
        };
        let config = self.ai_config()?;
        let model_digest = ollama::model_digest(&config)?;
        let key = ollama::key(&config, &model_digest, &inputs)?;
        let cache = self.root.join("ai").join(format!("{key}.json"));
        let advice = if cache.exists() {
            let mut cached: ollama::Assessment = self.load(&cache)?;
            ensure!(
                cached.input_sha256 == key && cached.model_digest == model_digest,
                "Cached model assessment does not match this evidence"
            );
            ollama::validate(&inputs, &cached.items)?;
            cached.cache_hit = true;
            cached
        } else {
            let result = ollama::assess(&config, &model_digest, &inputs)?;
            self.json(&cache, &result)?;
            result
        };
        job.ai = Some(advice);
        // Model advice never changes the comparison or reviewed selection.
        self.save(&mut job)?;
        Ok(job)
    }

    pub fn select(&self, id: &str, selected: Vec<String>) -> Result<Job> {
        let _guard = self.lock()?;
        let mut job = self.job(id)?;
        let comparison = job
            .comparison
            .as_ref()
            .context("Compare before reviewing corrections")?;
        let valid: HashSet<_> = comparison.edits.iter().map(|e| e.id.as_str()).collect();
        ensure!(
            selected.len() <= 5000 && selected.iter().all(|s| valid.contains(s.as_str())),
            "Selection contains a stale or invalid correction"
        );
        ensure!(
            selected.iter().collect::<HashSet<_>>().len() == selected.len(),
            "Duplicate selection"
        );
        job.selected = selected;
        self.save(&mut job)?;
        Ok(job)
    }

    pub fn export(&self, id: &str, directory: &Path) -> Result<PathBuf> {
        let _guard = self.lock()?;
        let mut job = self.job(id)?;
        let reference = job.reference.as_ref().context("No reference")?.clone();
        ensure!(reference.confirmed, "Confirm reference identity first");
        ensure!(
            book::file_hash(&reference.path)? == reference.sha256,
            "Reference changed; compare again"
        );
        let b = book::read(&job.book_path)?;
        ensure!(
            b.sha256 == job.input_sha256,
            "Original changed; analyze again"
        );
        let comparison = job.comparison.as_ref().context("No comparison")?;
        let edits: Vec<_> = comparison
            .edits
            .iter()
            .filter(|e| job.selected.contains(&e.id))
            .cloned()
            .collect();
        let name = job
            .book_path
            .file_stem()
            .context("No filename")?
            .to_string_lossy();
        let file = format!("{name}.corrected-{}.epub", uuid::Uuid::new_v4().simple());
        let output = directory.join(file);
        book::export(&b, &job.book_path, &output, &edits)?;
        job.exports.push(output.clone());
        self.save(&mut job)?;
        let report = output.with_extension("changes.json");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&report)?;
        serde_json::to_writer_pretty(
            &mut file,
            &serde_json::json!({
                "original_sha256": job.input_sha256, "reference": reference,
                "output": output, "changes": edits, "exported": now(),
            }),
        )?;
        file.sync_all()?;
        Ok(output)
    }
}
