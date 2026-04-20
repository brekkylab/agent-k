#!/bin/bash
# Download corpus, index, and config from S3 into backend/data/
#
# Prerequisites: aws cli configured with access to s3://ne-rag-dataset
#
# Usage: ./scripts/setup-data.sh

set -euo pipefail

BUCKET="s3://ne-rag-dataset"
DATA_DIR="backend/data"

# Resolve script location so it works from any CWD
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_PATH="$PROJECT_ROOT/$DATA_DIR"

echo "=== Setting up $DATA_PATH ==="

# 1. Config
echo "[1/4] Downloading knowledge_agents.json..."
mkdir -p "$DATA_PATH"
aws s3 cp "$BUCKET/agentwebui/knowledge_agents.json" "$DATA_PATH/knowledge_agents.json"

# 2. Corpus — finance (.md only, skip .pdf)
echo "[2/4] Downloading finance corpus..."
mkdir -p "$DATA_PATH/corpus/finance"
aws s3 sync "$BUCKET/PatronusAI__financebench/" "$DATA_PATH/corpus/finance/" \
  --exclude "*" --include "*.md"

# 3. Corpus — novel (.txt only) + QA file
echo "[3/4] Downloading novel corpus..."
mkdir -p "$DATA_PATH/corpus/novel"
aws s3 sync "$BUCKET/NovelQA__NovelQA/books/" "$DATA_PATH/corpus/novel/"
aws s3 cp "$BUCKET/NovelQA__NovelQA/novelqa_merged.json" "$DATA_PATH/corpus/novel/novelqa_merged.json"

# 4. Indexes
echo "[4/4] Downloading indexes..."
mkdir -p "$DATA_PATH/index/finance" "$DATA_PATH/index/novel"
aws s3 sync "$BUCKET/agentwebui/index/finance/" "$DATA_PATH/index/finance/"
aws s3 sync "$BUCKET/agentwebui/index/novel/" "$DATA_PATH/index/novel/"

echo ""
echo "=== Done ==="
echo "  Config:  $DATA_PATH/knowledge_agents.json"
echo "  Corpus:  $DATA_PATH/corpus/{finance,novel} (novel includes novelqa_merged.json)"
echo "  Index:   $DATA_PATH/index/{finance,novel}"
