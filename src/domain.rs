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
}

impl Entity {
    pub fn path(self) -> &'static str {
        match self {
            Entity::Skill => "skills",
            Entity::Category => "categories",
            Entity::Article => "articles",
            Entity::Tag => "tags",
            Entity::Resume => "resumes",
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
    SetArticlePublished {
        id: u64,
        publish: bool,
    },
    GetActiveResume,
    ActivateResume {
        id: u64,
    },
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
    fn tags_use_patch() {
        assert_eq!(Entity::Tag.update_method(), "PATCH");
        assert_eq!(Entity::Skill.update_method(), "PUT");
    }
}
