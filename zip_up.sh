#!/usr/bin/env bash
# This script zips up the contents of this repository into a zip file.

set -e
REPO_NAME=$(basename "$(git rev-parse --show-toplevel)")
ZIP_FILE="${REPO_NAME}.zip"
echo "Creating zip file: ${ZIP_FILE}"
git archive -o "${ZIP_FILE}" HEAD
echo "Zip file created successfully."


