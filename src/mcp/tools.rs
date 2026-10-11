//! MCP tool surface. Handlers only parse input, call the [`PortfolioApi`] port and map
//! the result; all business rules live in portfolio core.

use crate::application::{AppError, PortfolioApi};
use crate::domain::{validate_id, validate_page, Entity, Op};
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
    /// Article body (markdown/HTML as used by the site).
    pub content: String,
    pub tags: Option<Vec<String>>,
    pub excerpt: Option<String>,
    /// Absolute URL.
    pub cover_image: Option<String>,
    pub author_name: Option<String>,
    /// Absolute URL.
    pub author_avatar: Option<String>,
    /// `draft` (default) or `published`.
    pub status: Option<ArticleStatus>,
    /// ISO-8601 timestamp.
    pub published_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArticlePatch {
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
    pub status: Option<ArticleStatus>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArticleStatus {
    Draft,
    Published,
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
        description = "Create an article (draft by default)."
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
        self.run(
            validate_id(i.id)
                .map_err(AppError::Invalid)
                .map(|id| Op::SetArticlePublished {
                    id,
                    publish: i.publish,
                }),
        )
        .await
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
            .with_instructions("Admin tools for managing portfolio content (skills, categories, articles, tags, resumes).")
    }
}
