#!/bin/bash
set -e
[ -z "$1" ] && { echo "Usage: $0 <commit-message>"; exit 1; }

cargo build || {
     echo "Cargo build failed. Changes not committed. You may need to revert with 'git restore .' or similar.";
    exit 1
}
git add .
git commit -m "$1"
