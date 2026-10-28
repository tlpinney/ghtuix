//! The signed-in user.

use super::Client;
use anyhow::Result;
use graphql_client::GraphQLQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/viewer.graphql",
    response_derives = "Debug"
)]
struct Viewer;

/// The login of the account the token belongs to.
pub fn login(client: &Client) -> Result<String> {
    Ok(client.graphql::<Viewer>(viewer::Variables)?.viewer.login)
}
