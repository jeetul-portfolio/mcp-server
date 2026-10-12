//! MCP tool surface. Handlers only parse input, call the [`PortfolioApi`] port and map
//! the result; all business rules live in portfolio core.

use crate::application::{AppError, PortfolioApi};
use crate::domain::{id_or_slug, validate_id, validate_ids, validate_page, Entity, Op};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

// ---- input schemas -------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdInput {
    /// Record id (positive integer).
    pub id: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListInput {
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Page size, 1-500 (core applies a per-entity default).
    pub page_size: Option<u32>,
    /// Free-text filter.
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryListInput {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub search: Option<String>,
    /// Include each category's skills.
    pub include_skills: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArticleListInput {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub search: Option<String>,
    /// Filter by a single tag name.
    pub tag: Option<String>,
    /// Include draft articles (default false).
    pub include_drafts: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceType {
    Article,
    Experience,
    Project,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillReference {
    #[serde(rename = "type")]
    pub kind: ReferenceType,
    /// Id of the referenced article/experience/project.
    pub reference_id: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewSkill {
    /// Skill name (1-255 chars).
    pub name: String,
    pub category_ids: Option<Vec<u64>>,
    /// e.g. Beginner, Intermediate, Expert (max 50 chars; default Beginner).
    pub level: Option<String>,
    pub notes: Option<String>,
    pub sort_order: Option<u32>,
    pub references: Option<Vec<SkillReference>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillPatch {
    pub name: Option<String>,
    pub category_ids: Option<Vec<u64>>,
    pub level: Option<String>,
    pub notes: Option<String>,
    pub sort_order: Option<u32>,
    pub references: Option<Vec<SkillReference>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewCategory {
    /// Category name (1-100 chars).
    pub name: String,
    pub description: Option<String>,
    pub sort_order: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub sort_order: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewArticle {
    /// Title (1-255 chars).
    pub title: String,
    /// Article body (markdown/HTML as used by the site). `##tag` mentions also become tags.
    pub content: String,
    /// Author display name (1-120 chars). Required by core.
    pub author_name: String,
    /// Up to 50 tag names (1-80 chars each); names differing only in case/spacing are merged.
    pub tags: Option<Vec<String>>,
    pub excerpt: Option<String>,
    /// Absolute URL.
    pub cover_image: Option<String>,
    /// Absolute URL.
    pub author_avatar: Option<String>,
    /// `draft` (default) or `published`.
    pub status: Option<PublishStatus>,
    /// ISO-8601 timestamp.
    pub published_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArticlePatch {
    pub title: Option<String>,
    pub content: Option<String>,
    /// Replaces the whole tag list (up to 50 names, 1-80 chars each).
    pub tags: Option<Vec<String>>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    /// Author display name (1-120 chars); cannot be cleared.
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
    pub status: Option<PublishStatus>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PublishStatus {
    Draft,
    Published,
}

impl PublishStatus {
    fn as_str(self) -> &'static str {
        match self {
            PublishStatus::Draft => "draft",
            PublishStatus::Published => "published",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdOrSlugInput {
    /// Record id. Provide exactly one of id or slug.
    pub id: Option<u64>,
    /// Record slug (lowercase letters/digits separated by hyphens).
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReorderInput {
    /// Record ids in the desired order (unique); each gets sortOrder = its position.
    pub ids: Vec<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceListInput {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub search: Option<String>,
    /// Only records with this status (default: all, including drafts).
    pub status: Option<PublishStatus>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListInput {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub search: Option<String>,
    /// Only records with this status (default: all, including drafts).
    pub status: Option<PublishStatus>,
    pub featured: Option<bool>,
    /// Filter by a single tag name.
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExperienceLink {
    pub label: String,
    /// Absolute http(s) URL.
    pub url: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProjectLinkType {
    Repository,
    Demo,
    Documentation,
    Other,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProjectLink {
    #[serde(rename = "type")]
    pub kind: ProjectLinkType,
    pub label: String,
    /// Absolute http(s) URL.
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProjectImage {
    /// Absolute http(s) URL.
    pub url: String,
    pub alt: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewExperience {
    /// Job title.
    pub role: String,
    /// Organization name.
    pub company: String,
    /// Generated from role + company when omitted; must be unique.
    pub slug: Option<String>,
    /// Absolute http(s) URL.
    pub company_url: Option<String>,
    /// Absolute http(s) URL.
    pub logo_url: Option<String>,
    pub location: Option<String>,
    /// Free text, max 50 chars (default Full-time).
    pub employment_type: Option<String>,
    /// YYYY-MM-DD.
    pub start_date: Option<String>,
    /// YYYY-MM-DD, on or after startDate; ignored when isCurrent is true.
    pub end_date: Option<String>,
    pub is_current: Option<bool>,
    /// Short plain text.
    pub summary: Option<String>,
    /// Markdown.
    pub description: Option<String>,
    pub responsibilities: Option<Vec<String>>,
    pub achievements: Option<Vec<String>>,
    pub technologies: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    /// Ids of existing skills.
    pub skill_ids: Option<Vec<u64>>,
    pub links: Option<Vec<ExperienceLink>>,
    pub sort_order: Option<u32>,
    /// `draft` (default) or `published`.
    pub status: Option<PublishStatus>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExperiencePatch {
    pub role: Option<String>,
    pub company: Option<String>,
    pub slug: Option<String>,
    /// Absolute http(s) URL.
    pub company_url: Option<String>,
    /// Absolute http(s) URL.
    pub logo_url: Option<String>,
    pub location: Option<String>,
    pub employment_type: Option<String>,
    /// YYYY-MM-DD.
    pub start_date: Option<String>,
    /// YYYY-MM-DD.
    pub end_date: Option<String>,
    /// true clears endDate.
    pub is_current: Option<bool>,
    pub summary: Option<String>,
    /// Markdown.
    pub description: Option<String>,
    pub responsibilities: Option<Vec<String>>,
    pub achievements: Option<Vec<String>>,
    pub technologies: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    /// Replaces the whole skill set.
    pub skill_ids: Option<Vec<u64>>,
    pub links: Option<Vec<ExperienceLink>>,
    pub sort_order: Option<u32>,
    pub status: Option<PublishStatus>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    pub title: String,
    /// Generated from title when omitted; must be unique.
    pub slug: Option<String>,
    /// Short plain text.
    pub summary: Option<String>,
    /// Markdown.
    pub description: Option<String>,
    /// Markdown.
    pub problem: Option<String>,
    /// Markdown.
    pub solution: Option<String>,
    /// Markdown.
    pub technical_decisions: Option<String>,
    /// Markdown.
    pub outcomes: Option<String>,
    pub technologies: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    /// Ids of existing skills.
    pub skill_ids: Option<Vec<u64>>,
    pub links: Option<Vec<ProjectLink>>,
    /// Absolute http(s) URL.
    pub cover_image: Option<String>,
    pub images: Option<Vec<ProjectImage>>,
    pub is_featured: Option<bool>,
    pub sort_order: Option<u32>,
    /// `draft` (default) or `published`.
    pub status: Option<PublishStatus>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPatch {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub summary: Option<String>,
    /// Markdown.
    pub description: Option<String>,
    /// Markdown.
    pub problem: Option<String>,
    /// Markdown.
    pub solution: Option<String>,
    /// Markdown.
    pub technical_decisions: Option<String>,
    /// Markdown.
    pub outcomes: Option<String>,
    pub technologies: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    /// Replaces the whole skill set.
    pub skill_ids: Option<Vec<u64>>,
    pub links: Option<Vec<ProjectLink>>,
    /// Absolute http(s) URL.
    pub cover_image: Option<String>,
    pub images: Option<Vec<ProjectImage>>,
    pub is_featured: Option<bool>,
    pub sort_order: Option<u32>,
    pub status: Option<PublishStatus>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewTag {
    /// Tag name (1-100 chars).
    pub name: String,
    pub group: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
    pub is_seo_enabled: Option<bool>,
    pub is_internal: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagPatch {
    pub name: Option<String>,
    pub group: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
    pub is_seo_enabled: Option<bool>,
    pub is_internal: Option<bool>,
}

macro_rules! update_input {
    ($name:ident, $patch:ty) => {
        #[derive(Debug, Deserialize, JsonSchema)]
        pub struct $name {
            /// Id of the record to update.
            pub id: u64,
            /// Only the supplied fields are changed.
            #[serde(flatten)]
            pub patch: $patch,
        }
    };
}
update_input!(UpdateSkillInput, SkillPatch);
update_input!(UpdateCategoryInput, CategoryPatch);
update_input!(UpdateArticleInput, ArticlePatch);
update_input!(UpdateTagInput, TagPatch);
update_input!(UpdateExperienceInput, ExperiencePatch);
update_input!(UpdateProjectInput, ProjectPatch);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PublishInput {
    pub id: u64,
    /// true to publish, false to move back to draft.
    pub publish: bool,
}

// ---- server --------------------------------------------------------------------

#[derive(Clone)]
pub struct PortfolioServer {
    api: Arc<dyn PortfolioApi>,
    tool_router: ToolRouter<Self>,
}

/// Serializes input, dropping unset (null) top-level fields so core only sees what was supplied.
fn to_body<T: Serialize>(v: &T) -> Result<Value, AppError> {
    match serde_json::to_value(v).map_err(|_| AppError::Internal)? {
        Value::Object(map) => Ok(Value::Object(
            map.into_iter().filter(|(_, v)| !v.is_null()).collect(),
        )),
        other => Ok(other),
    }
}

fn list_query(
    page: Option<u32>,
    page_size: Option<u32>,
    search: Option<String>,
    mut extra: Vec<(&'static str, String)>,
) -> Result<Vec<(&'static str, String)>, AppError> {
    validate_page(page, page_size).map_err(AppError::Invalid)?;
    if let Some(p) = page {
        extra.push(("page", p.to_string()));
    }
    if let Some(s) = page_size {
        extra.push(("pageSize", s.to_string()));
    }
    if let Some(s) = search {
        extra.push(("search", s));
    }
    Ok(extra)
}

impl PortfolioServer {
    pub fn new(api: Arc<dyn PortfolioApi>) -> Self {
        Self {
            api,
            tool_router: Self::tool_router(),
        }
    }

    async fn run(&self, op: Result<Op, AppError>) -> CallToolResult {
        let result = match op {
            Ok(op) => self.api.execute(op).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(data) => CallToolResult::structured(json!({ "data": data })),
            Err(e) => CallToolResult::structured_error(json!({
                "error": e.code(),
                "message": e.to_string(),
            })),
        }
    }

    fn get(entity: Entity, id: u64) -> Result<Op, AppError> {
        Ok(Op::Get {
            entity,
            id: validate_id(id).map_err(AppError::Invalid)?,
        })
    }
    fn delete(entity: Entity, id: u64) -> Result<Op, AppError> {
        Ok(Op::Delete {
            entity,
            id: validate_id(id).map_err(AppError::Invalid)?,
        })
    }
    fn create<T: Serialize>(entity: Entity, body: &T) -> Result<Op, AppError> {
        Ok(Op::Create {
            entity,
            body: to_body(body)?,
        })
    }
    fn update<T: Serialize>(entity: Entity, id: u64, patch: &T) -> Result<Op, AppError> {
        let id = validate_id(id).map_err(AppError::Invalid)?;
        let body = to_body(patch)?;
        if body.as_object().is_some_and(|o| o.is_empty()) {
            return Err(AppError::Invalid(
                "provide at least one field to update".into(),
            ));
        }
        Ok(Op::Update { entity, id, body })
    }
    fn get_by(entity: Entity, i: IdOrSlugInput) -> Result<Op, AppError> {
        id_or_slug(entity, i.id, i.slug.as_deref()).map_err(AppError::Invalid)
    }
    fn set_published(entity: Entity, i: PublishInput) -> Result<Op, AppError> {
        Ok(Op::SetPublished {
            entity,
            id: validate_id(i.id).map_err(AppError::Invalid)?,
            publish: i.publish,
        })
    }
    fn reorder(entity: Entity, i: ReorderInput) -> Result<Op, AppError> {
        Ok(Op::Reorder {
            entity,
            ids: validate_ids(i.ids).map_err(AppError::Invalid)?,
        })
    }
}

#[tool_router]
impl PortfolioServer {
    // ---- skills
    #[tool(
        name = "portfolio_skills_list",
        description = "List skills (paginated, optional search).",
        annotations(read_only_hint = true)
    )]
    async fn skills_list(&self, Parameters(i): Parameters<ListInput>) -> CallToolResult {
        self.run(
            list_query(i.page, i.page_size, i.search, vec![]).map(|query| Op::List {
                entity: Entity::Skill,
                query,
            }),
        )
        .await
    }
    #[tool(
        name = "portfolio_skills_get",
        description = "Get one skill by id, including categories and references.",
        annotations(read_only_hint = true)
    )]
    async fn skills_get(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::get(Entity::Skill, i.id)).await
    }
    #[tool(
        name = "portfolio_skills_create",
        description = "Create a skill. Core validates the fields and links categories/references."
    )]
    async fn skills_create(&self, Parameters(i): Parameters<NewSkill>) -> CallToolResult {
        self.run(Self::create(Entity::Skill, &i)).await
    }
    #[tool(
        name = "portfolio_skills_update",
        description = "Update a skill by id. Only supplied fields change; supplying categoryIds/references replaces the whole set.",
        annotations(idempotent_hint = true)
    )]
    async fn skills_update(&self, Parameters(i): Parameters<UpdateSkillInput>) -> CallToolResult {
        self.run(Self::update(Entity::Skill, i.id, &i.patch)).await
    }
    #[tool(
        name = "portfolio_skills_delete",
        description = "Permanently delete a skill by id.",
        annotations(destructive_hint = true)
    )]
    async fn skills_delete(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::delete(Entity::Skill, i.id)).await
    }

    // ---- categories
    #[tool(
        name = "portfolio_categories_list",
        description = "List skill categories (paginated, optional search).",
        annotations(read_only_hint = true)
    )]
    async fn categories_list(
        &self,
        Parameters(i): Parameters<CategoryListInput>,
    ) -> CallToolResult {
        let extra = i
            .include_skills
            .map(|b| ("includeSkills", b.to_string()))
            .into_iter()
            .collect();
        self.run(
            list_query(i.page, i.page_size, i.search, extra).map(|query| Op::List {
                entity: Entity::Category,
                query,
            }),
        )
        .await
    }
    #[tool(
        name = "portfolio_categories_get",
        description = "Get one category by id.",
        annotations(read_only_hint = true)
    )]
    async fn categories_get(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::get(Entity::Category, i.id)).await
    }
    #[tool(
        name = "portfolio_categories_create",
        description = "Create a skill category."
    )]
    async fn categories_create(&self, Parameters(i): Parameters<NewCategory>) -> CallToolResult {
        self.run(Self::create(Entity::Category, &i)).await
    }
    #[tool(
        name = "portfolio_categories_update",
        description = "Update a category by id. Only supplied fields change.",
        annotations(idempotent_hint = true)
    )]
    async fn categories_update(
        &self,
        Parameters(i): Parameters<UpdateCategoryInput>,
    ) -> CallToolResult {
        self.run(Self::update(Entity::Category, i.id, &i.patch))
            .await
    }
    #[tool(
        name = "portfolio_categories_delete",
        description = "Permanently delete a category by id.",
        annotations(destructive_hint = true)
    )]
    async fn categories_delete(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::delete(Entity::Category, i.id)).await
    }

    // ---- articles
    #[tool(
        name = "portfolio_articles_list",
        description = "List articles (paginated; drafts excluded unless includeDrafts=true).",
        annotations(read_only_hint = true)
    )]
    async fn articles_list(&self, Parameters(i): Parameters<ArticleListInput>) -> CallToolResult {
        let mut extra = Vec::new();
        if let Some(t) = i.tag {
            extra.push(("tag", t));
        }
        if let Some(d) = i.include_drafts {
            extra.push(("includeDrafts", d.to_string()));
        }
        self.run(
            list_query(i.page, i.page_size, i.search, extra).map(|query| Op::List {
                entity: Entity::Article,
                query,
            }),
        )
        .await
    }
    #[tool(
        name = "portfolio_articles_get",
        description = "Get one article by id.",
        annotations(read_only_hint = true)
    )]
    async fn articles_get(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::get(Entity::Article, i.id)).await
    }
    #[tool(
        name = "portfolio_articles_create",
        description = "Create an article (draft by default). Requires title, content and authorName."
    )]
    async fn articles_create(&self, Parameters(i): Parameters<NewArticle>) -> CallToolResult {
        self.run(Self::create(Entity::Article, &i)).await
    }
    #[tool(
        name = "portfolio_articles_update",
        description = "Update an article by id. Only supplied fields change.",
        annotations(idempotent_hint = true)
    )]
    async fn articles_update(
        &self,
        Parameters(i): Parameters<UpdateArticleInput>,
    ) -> CallToolResult {
        self.run(Self::update(Entity::Article, i.id, &i.patch))
            .await
    }
    #[tool(
        name = "portfolio_articles_set_published",
        description = "Publish or unpublish an article by id.",
        annotations(idempotent_hint = true)
    )]
    async fn articles_set_published(
        &self,
        Parameters(i): Parameters<PublishInput>,
    ) -> CallToolResult {
        self.run(Self::set_published(Entity::Article, i)).await
    }
    #[tool(
        name = "portfolio_articles_delete",
        description = "Permanently delete an article by id.",
        annotations(destructive_hint = true)
    )]
    async fn articles_delete(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::delete(Entity::Article, i.id)).await
    }

    // ---- tags
    #[tool(
        name = "portfolio_tags_list",
        description = "List tags (paginated, optional search).",
        annotations(read_only_hint = true)
    )]
    async fn tags_list(&self, Parameters(i): Parameters<ListInput>) -> CallToolResult {
        self.run(
            list_query(i.page, i.page_size, i.search, vec![]).map(|query| Op::List {
                entity: Entity::Tag,
                query,
            }),
        )
        .await
    }
    #[tool(
        name = "portfolio_tags_get",
        description = "Get one tag by id.",
        annotations(read_only_hint = true)
    )]
    async fn tags_get(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::get(Entity::Tag, i.id)).await
    }
    #[tool(name = "portfolio_tags_create", description = "Create a tag.")]
    async fn tags_create(&self, Parameters(i): Parameters<NewTag>) -> CallToolResult {
        self.run(Self::create(Entity::Tag, &i)).await
    }
    #[tool(
        name = "portfolio_tags_update",
        description = "Update a tag by id. Only supplied fields change.",
        annotations(idempotent_hint = true)
    )]
    async fn tags_update(&self, Parameters(i): Parameters<UpdateTagInput>) -> CallToolResult {
        self.run(Self::update(Entity::Tag, i.id, &i.patch)).await
    }
    #[tool(
        name = "portfolio_tags_delete",
        description = "Permanently delete a tag by id.",
        annotations(destructive_hint = true)
    )]
    async fn tags_delete(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::delete(Entity::Tag, i.id)).await
    }

    // ---- experiences
    #[tool(
        name = "portfolio_experiences_list",
        description = "List work experiences, including drafts (paginated, optional search/status).",
        annotations(read_only_hint = true)
    )]
    async fn experiences_list(
        &self,
        Parameters(i): Parameters<ExperienceListInput>,
    ) -> CallToolResult {
        let extra = i
            .status
            .map(|s| ("status", s.as_str().to_owned()))
            .into_iter()
            .collect();
        self.run(
            list_query(i.page, i.page_size, i.search, extra).map(|query| Op::List {
                entity: Entity::Experience,
                query,
            }),
        )
        .await
    }
    #[tool(
        name = "portfolio_experiences_get",
        description = "Get one experience by id or slug (drafts included).",
        annotations(read_only_hint = true)
    )]
    async fn experiences_get(&self, Parameters(i): Parameters<IdOrSlugInput>) -> CallToolResult {
        self.run(Self::get_by(Entity::Experience, i)).await
    }
    #[tool(
        name = "portfolio_experiences_create",
        description = "Create a work experience (draft by default). Requires role and company."
    )]
    async fn experiences_create(&self, Parameters(i): Parameters<NewExperience>) -> CallToolResult {
        self.run(Self::create(Entity::Experience, &i)).await
    }
    #[tool(
        name = "portfolio_experiences_update",
        description = "Update an experience by id. Only supplied fields change; supplied arrays replace the whole list.",
        annotations(idempotent_hint = true)
    )]
    async fn experiences_update(
        &self,
        Parameters(i): Parameters<UpdateExperienceInput>,
    ) -> CallToolResult {
        self.run(Self::update(Entity::Experience, i.id, &i.patch))
            .await
    }
    #[tool(
        name = "portfolio_experiences_set_published",
        description = "Publish or unpublish an experience by id.",
        annotations(idempotent_hint = true)
    )]
    async fn experiences_set_published(
        &self,
        Parameters(i): Parameters<PublishInput>,
    ) -> CallToolResult {
        self.run(Self::set_published(Entity::Experience, i)).await
    }
    #[tool(
        name = "portfolio_experiences_reorder",
        description = "Set the display order of experiences.",
        annotations(idempotent_hint = true)
    )]
    async fn experiences_reorder(&self, Parameters(i): Parameters<ReorderInput>) -> CallToolResult {
        self.run(Self::reorder(Entity::Experience, i)).await
    }
    #[tool(
        name = "portfolio_experiences_delete",
        description = "Permanently delete an experience by id.",
        annotations(destructive_hint = true)
    )]
    async fn experiences_delete(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::delete(Entity::Experience, i.id)).await
    }

    // ---- projects
    #[tool(
        name = "portfolio_projects_list",
        description = "List projects, including drafts (paginated, optional search/status/featured/tag).",
        annotations(read_only_hint = true)
    )]
    async fn projects_list(&self, Parameters(i): Parameters<ProjectListInput>) -> CallToolResult {
        let mut extra = Vec::new();
        if let Some(s) = i.status {
            extra.push(("status", s.as_str().to_owned()));
        }
        if let Some(f) = i.featured {
            extra.push(("featured", f.to_string()));
        }
        if let Some(t) = i.tag {
            extra.push(("tag", t));
        }
        self.run(
            list_query(i.page, i.page_size, i.search, extra).map(|query| Op::List {
                entity: Entity::Project,
                query,
            }),
        )
        .await
    }
    #[tool(
        name = "portfolio_projects_get",
        description = "Get one project by id or slug (drafts included).",
        annotations(read_only_hint = true)
    )]
    async fn projects_get(&self, Parameters(i): Parameters<IdOrSlugInput>) -> CallToolResult {
        self.run(Self::get_by(Entity::Project, i)).await
    }
    #[tool(
        name = "portfolio_projects_create",
        description = "Create a project (draft by default). Requires title."
    )]
    async fn projects_create(&self, Parameters(i): Parameters<NewProject>) -> CallToolResult {
        self.run(Self::create(Entity::Project, &i)).await
    }
    #[tool(
        name = "portfolio_projects_update",
        description = "Update a project by id. Only supplied fields change; supplied arrays replace the whole list.",
        annotations(idempotent_hint = true)
    )]
    async fn projects_update(
        &self,
        Parameters(i): Parameters<UpdateProjectInput>,
    ) -> CallToolResult {
        self.run(Self::update(Entity::Project, i.id, &i.patch))
            .await
    }
    #[tool(
        name = "portfolio_projects_set_published",
        description = "Publish or unpublish a project by id.",
        annotations(idempotent_hint = true)
    )]
    async fn projects_set_published(
        &self,
        Parameters(i): Parameters<PublishInput>,
    ) -> CallToolResult {
        self.run(Self::set_published(Entity::Project, i)).await
    }
    #[tool(
        name = "portfolio_projects_reorder",
        description = "Set the display order of projects.",
        annotations(idempotent_hint = true)
    )]
    async fn projects_reorder(&self, Parameters(i): Parameters<ReorderInput>) -> CallToolResult {
        self.run(Self::reorder(Entity::Project, i)).await
    }
    #[tool(
        name = "portfolio_projects_delete",
        description = "Permanently delete a project by id.",
        annotations(destructive_hint = true)
    )]
    async fn projects_delete(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::delete(Entity::Project, i.id)).await
    }

    // ---- resumes (upload of new PDFs is intentionally not exposed: no filesystem access)
    #[tool(
        name = "portfolio_resumes_list",
        description = "List uploaded resumes.",
        annotations(read_only_hint = true)
    )]
    async fn resumes_list(&self) -> CallToolResult {
        self.run(Ok(Op::List {
            entity: Entity::Resume,
            query: vec![],
        }))
        .await
    }
    #[tool(
        name = "portfolio_resumes_get_active",
        description = "Get the currently active resume.",
        annotations(read_only_hint = true)
    )]
    async fn resumes_get_active(&self) -> CallToolResult {
        self.run(Ok(Op::GetActiveResume)).await
    }
    #[tool(
        name = "portfolio_resumes_activate",
        description = "Make a resume the active one by id.",
        annotations(idempotent_hint = true)
    )]
    async fn resumes_activate(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(
            validate_id(i.id)
                .map_err(AppError::Invalid)
                .map(|id| Op::ActivateResume { id }),
        )
        .await
    }
    #[tool(
        name = "portfolio_resumes_delete",
        description = "Permanently delete a resume by id.",
        annotations(destructive_hint = true)
    )]
    async fn resumes_delete(&self, Parameters(i): Parameters<IdInput>) -> CallToolResult {
        self.run(Self::delete(Entity::Resume, i.id)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PortfolioServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Admin tools for managing portfolio content (skills, categories, articles, tags, experiences, projects, resumes).")
    }
}
