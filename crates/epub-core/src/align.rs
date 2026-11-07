use crate::{book, digest, Comparison, Edit, Metadata};
use anyhow::{ensure, Result};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

pub fn normalize(s: &str) -> String {
    s.nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn similarity(a: &str, b: &str) -> f64 {
    let a = normalize(a);
    let b = normalize(b);
    let a: HashSet<_> = a.split_whitespace().collect();
    let b: HashSet<_> = b.split_whitespace().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / a.union(&b).count() as f64
}

pub fn identity(target: &Metadata, reference: &Metadata) -> Result<()> {
    ensure!(
        similarity(&target.title, &reference.title) >= 0.55,
        "Reference title does not match this work"
    );
    ensure!(
        target
            .authors
            .iter()
            .any(|a| reference.authors.iter().any(|b| similarity(a, b) >= 0.5)),
        "Reference author does not match; a title alone is insufficient"
    );
    if !target.language.is_empty() && !reference.language.is_empty() {
        ensure!(
            target.language.split('-').next() == reference.language.split('-').next(),
            "Reference language/translation differs"
        );
    }
    Ok(())
}

#[derive(Debug)]
struct Token {
    start: usize,
    end: usize,
    key: String,
}

fn tokens(text: &str) -> Vec<Token> {
    Regex::new(r"[\p{L}\p{N}]+(?:['’][\p{L}]+)*|[^\s\p{L}\p{N}]")
        .expect("constant expression")
        .find_iter(text)
        .map(|m| Token {
            start: m.start(),
            end: m.end(),
            key: m
                .as_str()
                .nfkc()
                .flat_map(char::to_lowercase)
                .map(|c| match c {
                    '’' => '\'',
                    '“' | '”' => '"',
                    _ => c,
                })
                .collect(),
        })
        .collect()
}

fn anchor_chain(a: &[Token], b: &[Token]) -> Vec<(usize, usize)> {
    const N: usize = 5;
    let key = |ts: &[Token]| {
        ts.iter()
            .map(|t| t.key.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}")
    };
    let mut index: HashMap<String, Option<usize>> = HashMap::new();
    for (i, ts) in b.windows(N).enumerate() {
        index
            .entry(key(ts))
            .and_modify(|v| *v = None)
            .or_insert(Some(i));
    }
    let candidates: Vec<_> = a
        .windows(N)
        .enumerate()
        .filter_map(|(i, ts)| index.get(&key(ts)).copied().flatten().map(|j| (i, j)))
        .collect();
    let mut tails: Vec<usize> = vec![];
    let mut previous = vec![None; candidates.len()];
    for (i, &(_, j)) in candidates.iter().enumerate() {
        let at = tails.partition_point(|&t| candidates[t].1 < j);
        if at > 0 {
            previous[i] = Some(tails[at - 1]);
        }
        if at == tails.len() {
            tails.push(i);
        } else {
            tails[at] = i;
        }
    }
    let mut chain = vec![];
    let mut cursor = tails.last().copied();
    while let Some(i) = cursor {
        chain.push(candidates[i]);
        cursor = previous[i];
    }
    chain.reverse();
    let mut spaced = vec![];
    for pair in chain {
        if spaced
            .last()
            .is_none_or(|&(i, j)| pair.0 >= i + N && pair.1 >= j + N)
        {
            spaced.push(pair);
        }
    }
    spaced
}

fn lcs(a: &[Token], b: &[Token], ai: usize, bi: usize) -> Vec<(usize, usize)> {
    let stride = b.len() + 1;
    let mut dp = vec![0u16; (a.len() + 1) * stride];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            dp[i * stride + j] = if a[i].key == b[j].key {
                1 + dp[(i + 1) * stride + j + 1]
            } else {
                dp[(i + 1) * stride + j].max(dp[i * stride + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut matches = vec![];
    while i < a.len() && j < b.len() {
        if a[i].key == b[j].key {
            matches.push((ai + i, bi + j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * stride + j] >= dp[i * stride + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matches
}

pub fn compare(target: &book::Book, reference: &str) -> Result<Comparison> {
    ensure!(
        reference.len() <= 16 * 1024 * 1024,
        "Reference text too large"
    );
    let a = tokens(&target.text);
    let b = tokens(reference);
    ensure!(
        a.len() <= 300_000 && b.len() <= 300_000,
        "Analysis token limit exceeded"
    );
    ensure!(
        a.len() >= 20 && b.len() >= 20,
        "Not enough text to establish matching content"
    );
    let anchors = anchor_chain(&a, &b);
    let coverage = (anchors.len() * 5) as f64 / a.len() as f64;
    ensure!(coverage >= 0.35 && anchors.len() >= 3,
        "Insufficient matching text ({:.1}% anchored); wrong edition, incomplete reference or unsupported damage",
        coverage * 100.0);
    let mut result = Comparison {
        anchor_count: anchors.len(), anchored_fraction: coverage,
        warning: "Differences are proposals, not proof of an OCR error. Review wording, edition and translation; no changes are selected automatically.".into(),
        ..Default::default()
    };
    let mut matches = vec![];
    let (mut ai, mut bi) = (0, 0);
    for &(i, j) in &anchors {
        if i - ai <= 160 && j - bi <= 160 {
            matches.extend(lcs(&a[ai..i], &b[bi..j], ai, bi));
        } else {
            result.skipped_regions += 1;
        }
        matches.extend((0..5).map(|k| (i + k, j + k)));
        ai = i + 5;
        bi = j + 5;
    }
    if a.len() - ai <= 160 && b.len() - bi <= 160 {
        matches.extend(lcs(&a[ai..], &b[bi..], ai, bi));
    } else {
        result.skipped_regions += 1;
    }
    let mut add = |start: usize, end: usize, after: &str, kind: &str| {
        let before = &target.text[start..end];
        if before == after || before.len() > 500 || after.len() > 500 || result.edits.len() >= 5000
        {
            return;
        }
        let id = digest(format!("{}:{start}:{end}:{before}:{after}", target.sha256).as_bytes());
        result.edits.push(Edit {
            id,
            start,
            end,
            before: before.into(),
            after: after.into(),
            context: book::context(&target.text, start, end),
            location: target.location(start),
            kind: kind.into(),
        });
    };
    for pair in matches.windows(2) {
        let ((i, j), (k, l)) = (pair[0], pair[1]);
        if k == i + 1 && l == j + 1 {
            let before = &target.text[a[i].end..a[k].start];
            let after = &reference[b[j].end..b[l].start];
            if before != after
                && !before.contains('\n')
                && !after.contains('\n')
                && before.len() <= 3
                && after.len() <= 3
            {
                add(a[i].end, a[k].start, after, "Reference-backed spacing");
            }
        } else if k - i <= 13 && l - j <= 13 {
            let start = if k == i + 1 { a[i].end } else { a[i + 1].start };
            let end = if k == i + 1 { start } else { a[k - 1].end };
            let after = if l == j + 1 {
                ""
            } else {
                &reference[b[j + 1].start..b[l - 1].end]
            };
            // Include reference boundary spaces for token insertion/deletion.
            if k == i + 1 && l > j + 1 {
                add(
                    a[i].end,
                    a[k].start,
                    &reference[b[j].end..b[l].start],
                    "Reference-backed text",
                );
            } else if l == j + 1 && k > i + 1 {
                add(
                    a[i].end,
                    a[k].start,
                    &reference[b[j].end..b[l].start],
                    "Reference-backed removal",
                );
            } else {
                add(start, end, after, "Reference-backed text");
            }
        }
    }
    result.edits.sort_by_key(|e| e.start);
    result
        .edits
        .dedup_by(|a, b| a.start == b.start && a.end == b.end && a.after == b.after);
    Ok(result)
}
