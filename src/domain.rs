//! Closed set of portfolio operations. Core owns the business rules; this layer only
//! names what the MCP server may ask core to do (no arbitrary paths or methods).

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    Skill,
    Category,
    Article,
    Tag,
    Resume,
    Experience,
    Project,
}

impl Entity {
    pub fn path(self) -> &'static str {
        match self {
            Entity::Skill => "skills",
            Entity::Category => "categories",
            Entity::Article => "articles",
            Entity::Tag => "tags",
            Entity::Resume => "resumes",
            Entity::Experience => "experiences",
            Entity::Project => "projects",
        }
    }

    /// Path for list/get. Experiences and projects are read through core's admin routes,
    /// which include drafts; the public routes only return published records.
    pub fn read_path(self) -> &'static str {
        match self {
            Entity::Experience => "admin/experiences",
            Entity::Project => "admin/projects",
            other => other.path(),
        }
    }

    /// Core uses PATCH for tags and PUT for the other editable entities.
    pub fn update_method(self) -> &'static str {
        match self {
            Entity::Tag => "PATCH",
            _ => "PUT",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Op {
    List {
        entity: Entity,
        query: Vec<(&'static str, String)>,
    },
    Get {
        entity: Entity,
        id: u64,
    },
    Create {
        entity: Entity,
        body: Value,
    },
    Update {
        entity: Entity,
        id: u64,
        body: Value,
    },
    Delete {
        entity: Entity,
        id: u64,
    },
    /// Read by slug (only entities whose core route accepts `:idOrSlug`).
    GetBySlug {
        entity: Entity,
        slug: Slug,
    },
    SetPublished {
        entity: Entity,
        id: u64,
        publish: bool,
    },
    /// Sets `sortOrder` to each id's position in `ids`.
    Reorder {
        entity: Entity,
        ids: Vec<u64>,
    },
    GetActiveResume,
    ActivateResume {
        id: u64,
    },
}

pub const MAX_SLUG_LEN: usize = 160;

/// A slug matching core's `^[a-z0-9]+(?:-[a-z0-9]+)*$` (max 160). Only constructible via
/// [`Slug::parse`], so it is always safe to place in a URL path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slug(String);

impl Slug {
    pub fn parse(s: &str) -> Result<Self, String> {
        let valid = !s.is_empty()
            && s.len() <= MAX_SLUG_LEN
            && s.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(format!(
                "slug must be lowercase letters/digits separated by single hyphens (max {MAX_SLUG_LEN} chars)"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exactly one of `id` / `slug` identifies the record.
pub fn id_or_slug(entity: Entity, id: Option<u64>, slug: Option<&str>) -> Result<Op, String> {
    match (id, slug) {
        (Some(id), None) => Ok(Op::Get {
            entity,
            id: validate_id(id)?,
        }),
        (None, Some(slug)) => Ok(Op::GetBySlug {
            entity,
            slug: Slug::parse(slug)?,
        }),
        _ => Err("provide exactly one of id or slug".into()),
    }
}

/// Reorder ids: non-empty, positive, no duplicates.
pub fn validate_ids(ids: Vec<u64>) -> Result<Vec<u64>, String> {
    if ids.is_empty() {
        return Err("ids must not be empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for &id in &ids {
        validate_id(id)?;
        if !seen.insert(id) {
            return Err(format!("duplicate id {id}"));
        }
    }
    Ok(ids)
}

/// Page bounds mirror core's Joi limits so obviously bad input is rejected locally.
pub const MAX_PAGE_SIZE: u32 = 500;

pub fn validate_id(id: u64) -> Result<u64, String> {
    if id == 0 {
        Err("id must be a positive integer".into())
    } else {
        Ok(id)
    }
}

pub fn validate_page(page: Option<u32>, page_size: Option<u32>) -> Result<(), String> {
    if page == Some(0) {
        return Err("page must be >= 1".into());
    }
    if let Some(size) = page_size {
        if size == 0 || size > MAX_PAGE_SIZE {
            return Err(format!("pageSize must be between 1 and {MAX_PAGE_SIZE}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_id_and_bad_pages() {
        assert!(validate_id(0).is_err());
        assert_eq!(validate_id(7), Ok(7));
        assert!(validate_page(Some(0), None).is_err());
        assert!(validate_page(None, Some(501)).is_err());
        assert!(validate_page(Some(1), Some(500)).is_ok());
    }

    #[test]
    fn slug_validation_matches_core_pattern() {
        for ok in [
            "a",
            "abc-123",
            "senior-backend-engineer-acme",
            &"a".repeat(160),
        ] {
            assert!(Slug::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-a",
            "a-",
            "a--b",
            "A",
            "a_b",
            "a/b",
            "../x",
            "a b",
            "é",
            &"a".repeat(161),
        ] {
            assert!(Slug::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn id_or_slug_requires_exactly_one() {
        let e = Entity::Project;
        assert!(id_or_slug(e, None, None).is_err());
        assert!(id_or_slug(e, Some(1), Some("x")).is_err());
        assert!(id_or_slug(e, Some(0), None).is_err());
        assert!(matches!(
            id_or_slug(e, Some(4), None),
            Ok(Op::Get { id: 4, .. })
        ));
        assert!(matches!(
            id_or_slug(e, None, Some("my-proj")),
            Ok(Op::GetBySlug { slug, .. }) if slug.as_str() == "my-proj"
        ));
    }

    #[test]
    fn reorder_ids_validated() {
        assert!(validate_ids(vec![]).is_err());
        assert!(validate_ids(vec![1, 0]).is_err());
        assert!(validate_ids(vec![1, 2, 1]).is_err());
        assert_eq!(validate_ids(vec![5, 2, 9]), Ok(vec![5, 2, 9]));
    }

    #[test]
    fn experiences_and_projects_read_via_admin_routes() {
        assert_eq!(Entity::Experience.read_path(), "admin/experiences");
        assert_eq!(Entity::Project.read_path(), "admin/projects");
        assert_eq!(Entity::Project.path(), "projects");
        assert_eq!(Entity::Article.read_path(), "articles");
    }

    #[test]
    fn tags_use_patch() {
        assert_eq!(Entity::Tag.update_method(), "PATCH");
        assert_eq!(Entity::Skill.update_method(), "PUT");
    }
}
