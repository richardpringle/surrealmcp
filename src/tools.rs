use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::tool::Parameters,
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use surrealdb::{Surreal, engine::any::Any};

use crate::validation::{Tier, classify_query};

#[derive(Deserialize, JsonSchema)]
pub struct QueryParams {
    #[schemars(description = "The SurrealQL query to execute")]
    pub sql: String,
    #[schemars(description = "Override the default namespace for this query")]
    pub namespace: Option<String>,
    #[schemars(description = "Override the default database for this query")]
    pub database: Option<String>,
}

#[derive(Clone)]
pub struct SurrealMcp {
    db: Surreal<Any>,
    default_ns: String,
    default_db: String,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SurrealMcp {
    pub fn new(db: Surreal<Any>, default_ns: String, default_db: String) -> Self {
        Self {
            db,
            default_ns,
            default_db,
            tool_router: Self::tool_router(),
        }
    }

    async fn execute(
        &self,
        max_tier: Tier,
        sql: &str,
        ns: Option<&str>,
        db: Option<&str>,
    ) -> Result<CallToolResult, McpError> {
        // Validate tier
        let (query_tier, keywords) = classify_query(sql);
        if query_tier > max_tier {
            return Err(McpError::invalid_params(
                format!(
                    "Query requires '{query_tier}' tier (found: {keywords:?}) \
                     but this tool only allows '{max_tier}'. \
                     Use the '{query_tier}' tool instead."
                ),
                None,
            ));
        }

        // Build query with USE prefix when overrides are specified
        let query = if ns.is_some() || db.is_some() {
            let ns = ns.unwrap_or(&self.default_ns);
            let db = db.unwrap_or(&self.default_db);
            format!("USE NS `{ns}` DB `{db}`; {sql}")
        } else {
            sql.to_string()
        };

        // Execute
        let mut response = self
            .db
            .query(&query)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        // Collect results, skipping the USE statement if we prepended one
        let skip = if ns.is_some() || db.is_some() { 1 } else { 0 };
        let num = response.num_statements();
        let mut results: Vec<serde_json::Value> = Vec::new();
        for i in skip..num {
            let result: Vec<serde_json::Value> = response
                .take(i)
                .map_err(|e| McpError::internal_error(format!("Statement {i}: {e}"), None))?;
            results.push(serde_json::Value::Array(result));
        }

        // Single-statement queries unwrap the outer array, multi-statement return array of arrays
        let output = if results.len() == 1 {
            serde_json::to_string_pretty(&results[0])
        } else {
            serde_json::to_string_pretty(&results)
        }
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(description = "Execute a read-only SurrealQL query. \
        Permits: SELECT, INFO, SHOW, LET, RETURN. \
        Rejects queries containing CREATE, INSERT, UPDATE, DELETE, or schema changes.")]
    pub async fn read(
        &self,
        params: Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        let QueryParams {
            sql,
            namespace,
            database,
        } = params.0;
        self.execute(Tier::Read, &sql, namespace.as_deref(), database.as_deref())
            .await
    }

    #[tool(description = "Execute a SurrealQL query that creates new records. \
        Permits: SELECT, INFO, SHOW, LET, RETURN, CREATE, INSERT, RELATE. \
        Rejects queries containing UPDATE, DELETE, or schema changes.")]
    pub async fn create(
        &self,
        params: Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        let QueryParams {
            sql,
            namespace,
            database,
        } = params.0;
        self.execute(
            Tier::Create,
            &sql,
            namespace.as_deref(),
            database.as_deref(),
        )
        .await
    }

    #[tool(description = "Execute a SurrealQL query that modifies existing records. \
        Permits: all read/create statements plus UPDATE, UPSERT. \
        Rejects queries containing DELETE or schema changes.")]
    pub async fn mutate(
        &self,
        params: Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        let QueryParams {
            sql,
            namespace,
            database,
        } = params.0;
        self.execute(
            Tier::Mutate,
            &sql,
            namespace.as_deref(),
            database.as_deref(),
        )
        .await
    }

    #[tool(description = "Execute a SurrealQL query that deletes records. \
        Permits: all read/create/mutate statements plus DELETE, REMOVE. \
        Rejects schema changes (DEFINE, ALTER, etc).")]
    pub async fn delete(
        &self,
        params: Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        let QueryParams {
            sql,
            namespace,
            database,
        } = params.0;
        self.execute(
            Tier::Delete,
            &sql,
            namespace.as_deref(),
            database.as_deref(),
        )
        .await
    }

    #[tool(description = "Execute any SurrealQL query with no restrictions. \
        Permits all statement types including DEFINE, ALTER, REBUILD, \
        USE, BEGIN/COMMIT, LIVE SELECT, and control flow.")]
    pub async fn admin(
        &self,
        params: Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        let QueryParams {
            sql,
            namespace,
            database,
        } = params.0;
        self.execute(
            Tier::Admin,
            &sql,
            namespace.as_deref(),
            database.as_deref(),
        )
        .await
    }
}

#[tool_handler]
impl ServerHandler for SurrealMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "SurrealDB MCP server with hierarchical permission-tiered query tools. \
                 Use 'read' for SELECT queries, 'create' for INSERT/CREATE/RELATE, \
                 'mutate' for UPDATE/UPSERT, 'delete' for DELETE/REMOVE, \
                 and 'admin' for schema changes and control flow."
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}
