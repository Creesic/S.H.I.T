#!/bin/bash
# Auto-increment version and build

cd "$(dirname "$0")"

# Read current version
VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')

# Split version into parts
IFS='.' read -r MAJOR MINOR PATCH <<< "$VERSION"

# Increment patch version
PATCH=$((PATCH + 1))

# Handle overflow (e.g., 0.1.9 -> 0.2.0)
if [ $PATCH -ge 100 ]; then
    PATCH=0
    MINOR=$((MINOR + 1))
fi

NEW_VERSION="$MAJOR.$MINOR.$PATCH"

# Update Cargo.toml
sed -i '' "s/^version = \"$VERSION\"/version = \"$NEW_VERSION\"/" Cargo.toml

echo "Building version $NEW_VERSION..."

# Build and run
cargo run --release
