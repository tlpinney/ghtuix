#!/bin/sh
# Refresh GitHub's public GraphQL schema; queries are checked against it at build time.
set -eu
cd "$(dirname "$0")/.."
curl -sSfL -o schema/github.graphql https://docs.github.com/public/fpt/schema.docs.graphql
echo "Updated schema/github.graphql; run cargo build to check every query against it."
