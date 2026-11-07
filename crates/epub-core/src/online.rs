use crate::{align, Metadata};
use anyhow::{bail, ensure, Context, Result};
use reqwest::{blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Read,
    net::{IpAddr, ToSocketAddrs},
    time::Duration,
};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub provider: String,
    pub metadata: Metadata,
    pub page_url: String,
    pub download_url: Option<String>,
    pub format: String,
    pub notice: String,
    pub identity_match: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Search {
    pub candidates: Vec<Candidate>,
    pub notices: Vec<String>,
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            !a.is_private()
                && !a.is_loopback()
                && !a.is_link_local()
                && !a.is_unspecified()
                && !a.is_broadcast()
                && !a.is_multicast()
                && !a.is_documentation()
                && a.octets()[0] != 0
                && a.octets()[0] < 224
                && !(a.octets()[0] == 100 && (64..128).contains(&a.octets()[1]))
        }
        IpAddr::V6(a) => {
            let first = a.segments()[0];
            first & 0xe000 == 0x2000 && !(first == 0x2001 && a.segments()[1] == 0x0db8)
        }
    }
}

pub fn validate_url(u: &Url) -> Result<()> {
    ensure!(
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443),
        "Only uncredentialed HTTPS on port 443 is supported"
    );
    ensure!(matches!(u.host_str(), Some("gutendex.com" | "openlibrary.org" | "gutenberg.org" | "www.gutenberg.org")),
        "This source is not an enabled trusted provider; import an acquired reference locally instead");
    Ok(())
}

pub fn get(url: &str, max: u64) -> Result<Vec<u8>> {
    let mut url = Url::parse(url)?;
    for _ in 0..4 {
        validate_url(&url)?;
        let host = url.host_str().context("No source host")?;
        let addresses: Vec<_> = (host, 443).to_socket_addrs()?.collect();
        ensure!(
            !addresses.is_empty() && addresses.iter().all(|a| public(a.ip())),
            "Provider DNS did not resolve exclusively to public addresses"
        );
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(if max <= 4 * 1024 * 1024 {
                15
            } else {
                45
            }))
            .resolve_to_addrs(host, &addresses)
            .user_agent("EPUB-Reference-Studio/0.1 (desktop reference research)")
            .build()?;
        let response = client.get(url.clone()).send()?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .context("Redirect has no destination")?
                .to_str()?;
            url = url.join(location)?;
            continue;
        }
        let response = response.error_for_status()?;
        ensure!(
            response.content_length().is_none_or(|n| n <= max),
            "Provider response too large"
        );
        let mut bytes = vec![];
        response.take(max + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= max,
            "Provider response exceeded size limit"
        );
        return Ok(bytes);
    }
    bail!("Provider redirect limit exceeded")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

pub fn parse_opds(meta: &Metadata, source: &str) -> Result<Vec<Candidate>> {
    let doc = crate::book::xml(source)?;
    let mut candidates = vec![];
    for entry in doc
        .descendants()
        .filter(|n| n.has_tag_name("entry"))
        .take(40)
    {
        let field = |name: &str| {
            entry
                .children()
                .find(|n| n.has_tag_name(name))
                .and_then(|n| n.text())
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        let id_url = Url::parse(&field("id"))?;
        validate_url(&id_url)?;
        let Some(id) = id_url
            .path()
            .strip_prefix("/ebooks/")
            .and_then(|s| s.strip_suffix(".opds"))
        else {
            continue;
        };
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let author = field("content");
        let metadata = Metadata {
            title: field("title"),
            authors: if author.is_empty() {
                vec![]
            } else {
                vec![author]
            },
            language: String::new(),
            identifiers: vec![format!("gutenberg:{id}")],
        };
        candidates.push(Candidate {
            id: format!("pg-{id}"), provider: "Project Gutenberg official OPDS".into(),
            page_url: format!("https://www.gutenberg.org/ebooks/{id}"),
            download_url: Some(format!("https://www.gutenberg.org/ebooks/{id}.epub3.images")),
            format: "application/epub+zip".into(),
            identity_match: align::identity(meta, &metadata).is_ok(), metadata,
            notice: "Official catalogue candidate, not a certified identical edition. Review Gutenberg's licence, translation and source edition; the downloaded EPUB's own metadata is checked again.".into(),
        });
    }
    Ok(candidates)
}

pub fn search(meta: &Metadata, query: &str) -> Search {
    let mut result = Search::default();
    let query = if query.trim().is_empty() {
        meta.title.as_str()
    } else {
        query.trim()
    };
    let mut feed =
        Url::parse("https://www.gutenberg.org/ebooks/search.opds/").expect("constant URL");
    feed.query_pairs_mut().append_pair("query", query);
    match get(feed.as_str(), 4 * 1024 * 1024)
        .and_then(|b| Ok(String::from_utf8(b)?))
        .and_then(|text| parse_opds(meta, &text))
    {
        Ok(candidates) => result.candidates.extend(candidates),
        Err(e) => result
            .notices
            .push(format!("Gutenberg OPDS unavailable: {e:#}")),
    }
    if !result
        .candidates
        .iter()
        .any(|c| c.identity_match && c.download_url.is_some())
    {
        let mut pg = Url::parse("https://gutendex.com/books/").expect("constant URL");
        pg.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("copyright", "false");
        match get(pg.as_str(), 4 * 1024 * 1024)
            .and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?))
        {
            Ok(v) => {
                for item in v["results"].as_array().into_iter().flatten().take(32) {
                    let metadata = Metadata {
                        title: item["title"].as_str().unwrap_or("").into(),
                        authors: item["authors"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|a| a["name"].as_str())
                            .map(str::to_owned)
                            .collect(),
                        language: strings(&item["languages"])
                            .first()
                            .cloned()
                            .unwrap_or_default(),
                        identifiers: vec![format!("gutenberg:{}", item["id"])],
                    };
                    let formats = &item["formats"];
                    let choice = [
                        "application/epub+zip",
                        "text/plain; charset=utf-8",
                        "text/plain",
                    ]
                    .into_iter()
                    .find_map(|key| formats[key].as_str().map(|s| (key, s)));
                    let (format, download_url) = match choice {
                        Some((f, s)) => {
                            let mut u = Url::parse(s).ok();
                            if let Some(u) = &mut u {
                                if u.scheme() == "http" {
                                    let _ = u.set_scheme("https");
                                }
                            }
                            (
                                f.to_owned(),
                                u.filter(|u| validate_url(u).is_ok()).map(|u| u.to_string()),
                            )
                        }
                        None => ("unavailable".into(), None),
                    };
                    result.candidates.push(Candidate {
                    id: format!("pg-{}", item["id"]), provider: "Project Gutenberg / Gutendex".into(),
                    page_url: format!("https://www.gutenberg.org/ebooks/{}", item["id"]),
                    identity_match: align::identity(meta, &metadata).is_ok(), metadata, download_url, format,
                    notice: "Catalogue reports US public-domain status. Confirm work, translation, edition and applicable local rights. Availability is not textual certification.".into(),
                });
                }
            }
            Err(e) => result
                .notices
                .push(format!("Gutenberg search unavailable: {e:#}")),
        }
    }
    // Open Library provides bibliographic leads, not an unrestricted ebook API.
    std::thread::sleep(Duration::from_secs(1));
    let mut ol = Url::parse("https://openlibrary.org/search.json").expect("constant URL");
    ol.query_pairs_mut()
        .append_pair("title", query)
        .append_pair(
            "author",
            meta.authors.first().map(String::as_str).unwrap_or(""),
        )
        .append_pair("limit", "10")
        .append_pair(
            "fields",
            "key,title,author_name,language,isbn,first_publish_year,ebook_access",
        );
    match get(ol.as_str(), 2 * 1024 * 1024).and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?)) {
        Ok(v) => {
            for item in v["docs"].as_array().into_iter().flatten().take(10) {
                let key = item["key"].as_str().unwrap_or("");
                if !key.starts_with("/works/") || key.contains(['?', '#', '\\']) {
                    continue;
                }
                let metadata = Metadata {
                    title: item["title"].as_str().unwrap_or("").into(),
                    authors: strings(&item["author_name"]),
                    language: String::new(),
                    identifiers: strings(&item["isbn"]).into_iter().take(5).collect(),
                };
                result.candidates.push(Candidate {
                    id: key.into(), provider: "Open Library catalogue".into(),
                    page_url: format!("https://openlibrary.org{key}"), download_url: None,
                    format: "Bibliographic lead — no downloadable text supplied".into(),
                    identity_match: align::identity(meta, &metadata).is_ok(), metadata,
                    notice: "Metadata is not reference text. Use this lead to identify an accessible matching edition, then import your acquired EPUB/text.".into(),
                });
            }
        }
        Err(e) => result
            .notices
            .push(format!("Open Library search unavailable: {e:#}")),
    }
    let mut seen = HashSet::new();
    result.candidates.retain(|c| seen.insert(c.id.clone()));
    if !result
        .candidates
        .iter()
        .any(|c| c.download_url.is_some() && c.identity_match)
    {
        result.notices.push("No matching downloadable full text found. Do not substitute an unrelated book or generate missing text. Import an accessible matching reference EPUB or UTF-8 text.".into());
    }
    result
}
