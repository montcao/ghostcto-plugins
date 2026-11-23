//! Which GhostCTO profiles are installed, and the category each belongs to.
//!
//! Profiles are discovered at runtime from the GhostCTO Claude Code plugin and
//! from standalone `ghostcto-*` skills. Display names come from each SKILL.md's
//! first heading, so they follow the profile text. Categories mirror the
//! GhostCTO catalog; a profile missing from the table shows as `Unlisted`.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Build,
    Quality,
    Communication,
    Unlisted,
}

/// Mirrors `category` in the GhostCTO catalog (src/shared/profiles/catalog.ts).
/// Order is the order ghosts appear in: Big Boss first.
const CATALOG: &[(&str, Category)] = &[
    ("bigboss", Category::Build),
    ("developer", Category::Build),
    ("test-writer", Category::Quality),
    ("docs-writer", Category::Communication),
    ("issue-triager", Category::Communication),
    ("pr-summarizer", Category::Communication),
    ("lint-fixer", Category::Quality),
    ("security", Category::Quality),
    ("cicd-infrastructure", Category::Build),
    ("committer", Category::Quality),
    ("harness-engineer", Category::Build),
    ("mcp-apps-specialist", Category::Build),
    ("web-specialist", Category::Build),
    ("desktop-specialist", Category::Build),
    ("web-designer", Category::Build),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub slug: String,
    pub name: String,
    pub category: Category,
    /// The SKILL.md `description`: what the profile is for.
    pub summary: String,
}

impl Profile {
    pub fn from_slug(slug: &str) -> Self {
        Profile {
            slug: slug.to_string(),
            name: title_case(slug),
            category: category_of(slug),
            summary: String::new(),
        }
    }
}

fn category_of(slug: &str) -> Category {
    CATALOG
        .iter()
        .find(|(s, _)| *s == slug)
        .map(|(_, c)| *c)
        .unwrap_or(Category::Unlisted)
}

fn catalog_rank(slug: &str) -> usize {
    CATALOG
        .iter()
        .position(|(s, _)| *s == slug)
        .unwrap_or(CATALOG.len())
}

/// Maps a skill name as Claude Code reports it to a profile slug:
/// `ghostcto:committer` (plugin) and `ghostcto-committer` (standalone skill)
/// both become `committer`; the plugin's `start` is Big Boss.
pub fn slug_for_skill(skill: &str) -> Option<String> {
    let name = skill
        .strip_prefix("ghostcto:")
        .or_else(|| skill.strip_prefix("ghostcto-"))?;
    plugin_skill_to_slug(name)
}

fn plugin_skill_to_slug(name: &str) -> Option<String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        return None;
    }
    Some(if name == "start" {
        "bigboss".to_string()
    } else {
        name.to_string()
    })
}

fn title_case(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The frontmatter `description` of a SKILL.md, unquoted.
fn description(text: &str) -> Option<String> {
    let body = text.strip_prefix("---\n")?;
    let frontmatter = &body[..body.find("\n---")?];
    frontmatter
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(|d| d.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|d| !d.is_empty())
}

/// The first sentence of a description: what the profile is best for.
fn first_sentence(text: &str) -> String {
    match text.find(". ") {
        Some(end) => text[..=end].to_string(),
        None => text.to_string(),
    }
}

/// First `# ` heading of a SKILL.md, the profile's display name.
fn heading(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("# "))
        .map(|h| h.trim().trim_start_matches("GhostCTO ").to_string())
        .filter(|h| !h.is_empty() && h.len() <= 40)
}

/// Install directory of the `ghostcto` plugin, read from Claude Code's
/// installed_plugins.json.
fn plugin_dir(claude_home: &Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(claude_home.join("plugins/installed_plugins.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let plugins = json.get("plugins").unwrap_or(&json).as_object()?;
    plugins
        .iter()
        .find(|(key, _)| key.starts_with("ghostcto@"))
        .and_then(|(_, v)| {
            v.as_array()?
                .first()?
                .get("installPath")?
                .as_str()
                .map(PathBuf::from)
        })
}

/// Every GhostCTO profile installed for this user, Big Boss first.
pub fn discover(claude_home: &Path) -> Vec<Profile> {
    let mut found: Vec<Profile> = Vec::new();
    let mut add = |slug: String, skill_md: PathBuf| {
        if found.iter().any(|p| p.slug == slug) {
            return;
        }
        let mut profile = Profile::from_slug(&slug);
        let text = fs::read_to_string(&skill_md).unwrap_or_default();
        if let Some(name) = heading(&text) {
            profile.name = name;
        }
        if let Some(summary) = description(&text) {
            profile.summary = first_sentence(&summary);
        }
        found.push(profile);
    };

    if let Some(dir) = plugin_dir(claude_home) {
        for entry in read_dir_sorted(&dir.join("skills")) {
            if let Some(slug) = entry
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(plugin_skill_to_slug)
            {
                add(slug, entry.join("SKILL.md"));
            }
        }
    }
    for entry in read_dir_sorted(&claude_home.join("skills")) {
        if let Some(slug) = entry
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(slug_for_skill)
        {
            add(slug, entry.join("SKILL.md"));
        }
    }
    found.sort_by_key(|p| (catalog_rank(&p.slug), p.slug.clone()));
    found
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_names_map_to_slugs() {
        assert_eq!(
            slug_for_skill("ghostcto:committer").as_deref(),
            Some("committer")
        );
        assert_eq!(
            slug_for_skill("ghostcto-committer").as_deref(),
            Some("committer")
        );
        assert_eq!(slug_for_skill("ghostcto:start").as_deref(), Some("bigboss"));
        assert_eq!(slug_for_skill("ghostcto:Bad Name"), None);
        assert_eq!(slug_for_skill("ghostcto:../x"), None);
        assert_eq!(slug_for_skill("superpowers:brainstorm"), None);
    }

    #[test]
    fn unknown_slug_is_unlisted_and_title_cased() {
        let p = Profile::from_slug("data-wrangler");
        assert_eq!(p.category, Category::Unlisted);
        assert_eq!(p.name, "Data Wrangler");
        assert_eq!(Profile::from_slug("committer").category, Category::Quality);
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn temp_home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ghostcto-herdr-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discovers_plugin_and_standalone_profiles() {
        let home = temp_home("discover");
        let install = home.join("plugins/cache/ghostcto/abc");
        write(
            &home.join("plugins/installed_plugins.json"),
            &format!(
                r#"{{"version":2,"plugins":{{"ghostcto@ghostcto-profiles":[{{"installPath":{:?}}}]}}}}"#,
                install
            ),
        );
        write(
            &install.join("skills/committer/SKILL.md"),
            "---\nname: ghostcto-committer\n---\n\n# Committer\n",
        );
        write(
            &install.join("skills/start/SKILL.md"),
            "---\nname: ghostcto-bigboss\n---\n\n# Big Boss\n",
        );
        write(
            &home.join("skills/ghostcto-web-designer/SKILL.md"),
            "# Web Designer\n",
        );
        write(
            &home.join("skills/ghostcto-committer/SKILL.md"),
            "# Duplicate\n",
        );
        write(&home.join("skills/other-skill/SKILL.md"), "# Other\n");

        let found = discover(&home);
        let names: Vec<_> = found
            .iter()
            .map(|p| (p.slug.as_str(), p.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("bigboss", "Big Boss"),
                ("committer", "Committer"),
                ("web-designer", "Web Designer")
            ]
        );
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn summary_is_the_first_sentence() {
        assert_eq!(
            first_sentence("Act as the CTO. Turn a vague goal into a plan."),
            "Act as the CTO."
        );
        assert_eq!(
            first_sentence("Commit finished work"),
            "Commit finished work"
        );
        assert_eq!(
            first_sentence("Uses v1.2 rules. Then stops."),
            "Uses v1.2 rules."
        );
    }

    #[test]
    fn nothing_installed_is_empty() {
        let home = temp_home("empty");
        assert!(discover(&home).is_empty());
        fs::remove_dir_all(&home).unwrap();
    }
}
