# Resolves the CortexDB container reference for Compose.
#
# The default is the original public v0.10.4 OCI index, pinned by digest so it
# remains pullable even when Docker Hub removes the version tag. Existing
# scripts' CORTEXDB_VERSION override still selects a tag; CORTEXDB_IMAGE can
# select an explicit complete image reference instead.
if [[ -z "${CORTEXDB_IMAGE:-}" ]]; then
  if [[ -n "${CORTEXDB_VERSION:-}" ]]; then
    CORTEXDB_IMAGE="cortexdb/cortexdb:${CORTEXDB_VERSION}"
  else
    CORTEXDB_IMAGE="cortexdb/cortexdb@sha256:636e0ed0b8cd4ae2f307e3a90f99050f7879f73c556b1ccbbabf2d3a383398fc"
  fi
fi
export CORTEXDB_IMAGE
