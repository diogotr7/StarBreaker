#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
ARTIFACT_DIR="${REPO_ROOT}/test-artifacts/ui"
FREEZE_FILE="${UI_REGRESSION_FREEZE_PATH:-${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json}"
SNAPSHOT_FREEZE_FILE="${UI_SNAPSHOT_FREEZE_PATH:-${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json}"
VALIDATION_MODE="full"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --quick)
      VALIDATION_MODE="quick"
      shift
      ;;
    --full)
      VALIDATION_MODE="full"
      shift
      ;;
    *)
      echo "error: unknown argument: $1 (expected --quick or --full)" >&2
      exit 1
      ;;
  esac
done

mapfile -t MANIFEST_CANDIDATES < <(find "${REPO_ROOT}/crates/starbreaker-ui/tests/fixtures" -type f -name '*snapshot_manifest.json' | sort)
DEFAULT_MANIFEST_PATH=""
if [[ "${#MANIFEST_CANDIDATES[@]}" -eq 1 ]]; then
  DEFAULT_MANIFEST_PATH="${MANIFEST_CANDIDATES[0]}"
fi
MANIFEST_PATH="${UI_REGRESSION_MANIFEST_PATH:-${DEFAULT_MANIFEST_PATH}}"

# Inputs before environment: a missing freeze file is reported even where
# the full-mode tools (magick, sha256sum) or local artifacts are absent.
if [[ "${VALIDATION_MODE}" == "full" && ! -f "${FREEZE_FILE}" ]]; then
  echo "error: freeze file not found: ${FREEZE_FILE}" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required but not installed" >&2
  exit 1
fi

if [[ "${VALIDATION_MODE}" == "full" ]]; then
  if ! command -v magick >/dev/null 2>&1; then
    echo "error: ImageMagick 'magick' command is required but not installed" >&2
    exit 1
  fi

  if ! command -v sha256sum >/dev/null 2>&1; then
    echo "error: sha256sum is required but not installed" >&2
    exit 1
  fi
fi

if [[ -z "${MANIFEST_PATH}" || ! -f "${MANIFEST_PATH}" ]]; then
  echo "error: UI regression manifest not found: ${MANIFEST_PATH}" >&2
  if [[ "${#MANIFEST_CANDIDATES[@]}" -gt 1 ]]; then
    echo "error: multiple manifest candidates found; pass UI_REGRESSION_MANIFEST_PATH" >&2
    for candidate in "${MANIFEST_CANDIDATES[@]}"; do
      echo "  - ${candidate}" >&2
    done
  fi
  exit 1
fi

if [[ "${VALIDATION_MODE}" == "full" && ! -d "${ARTIFACT_DIR}" ]]; then
  echo "error: artifact directory missing: ${ARTIFACT_DIR}" >&2
  exit 1
fi

if ! jq -e '.schema_version == 1 and (.targets | type == "array")' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest schema/targets validation failed: ${MANIFEST_PATH}" >&2
  exit 1
fi

if ! jq -e '(.targets | map(.id)) as $ids | ($ids | length) == ($ids | unique | length)' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest contains duplicate target ids: ${MANIFEST_PATH}" >&2
  exit 1
fi

if ! jq -e '.targets | all(.tier == "gold" or .tier == "platinum")' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: manifest contains invalid tier values (expected gold/platinum)" >&2
  exit 1
fi

if ! jq -e '.targets | all(.source_generated_png != null and (.source_generated_png | type == "string") and (.source_generated_png | length > 0))' "${MANIFEST_PATH}" >/dev/null 2>&1; then
  echo "error: every target must include non-empty source_generated_png" >&2
  exit 1
fi

if [[ ! -f "${SNAPSHOT_FREEZE_FILE}" ]]; then
  echo "error: snapshot freeze file not found: ${SNAPSHOT_FREEZE_FILE}" >&2
  exit 1
fi

UI_REGRESSION_MANIFEST_PATH="${MANIFEST_PATH}" \
UI_SNAPSHOT_FREEZE_PATH="${SNAPSHOT_FREEZE_FILE}" \
  bash "${SCRIPT_DIR}/validate_ui_snapshot_freeze.sh"

errors=0
checked=0

if [[ "${VALIDATION_MODE}" == "full" ]]; then
  while IFS=$'\t' read -r target_id source_png tier; do
    [[ -n "${target_id}" ]] || continue
    checked=$((checked + 1))

    if [[ "${source_png}" = /* ]]; then
      source_path="${source_png}"
    elif [[ "${source_png}" = ships/* ]]; then
      source_path="${WORKSPACE_ROOT}/${source_png}"
    else
      source_path="${REPO_ROOT}/${source_png}"
    fi

    artifact_path="${ARTIFACT_DIR}/${target_id}.png"

  if [[ ! -f "${source_path}" ]]; then
    echo "error: missing source image for ${target_id}: ${source_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  if [[ ! -f "${artifact_path}" ]]; then
    echo "error: missing artifact image for ${target_id}: ${artifact_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  source_meta="$(magick identify -format '%w %h %[channels]' "${source_path}" 2>/dev/null || true)"
  artifact_meta="$(magick identify -format '%w %h %[channels]' "${artifact_path}" 2>/dev/null || true)"

  if [[ -z "${source_meta}" ]]; then
    echo "error: failed to inspect source image for ${target_id}: ${source_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  if [[ -z "${artifact_meta}" ]]; then
    echo "error: failed to inspect artifact image for ${target_id}: ${artifact_path}" >&2
    errors=$((errors + 1))
    continue
  fi

  read -r src_w src_h src_channels <<< "${source_meta}"
  read -r out_w out_h out_channels <<< "${artifact_meta}"
  src_channels="$(echo "${src_channels}" | awk '{print tolower($1)}')"
  out_channels="$(echo "${out_channels}" | awk '{print tolower($1)}')"

  if [[ "${src_w}" != "${out_w}" || "${src_h}" != "${out_h}" ]]; then
    echo "error: dimension mismatch for ${target_id}: source=${src_w}x${src_h} artifact=${out_w}x${out_h}" >&2
    errors=$((errors + 1))
  fi

  if [[ "${src_channels}" != "${out_channels}" ]]; then
    echo "error: channel mismatch for ${target_id}: source=${src_channels} artifact=${out_channels}" >&2
    errors=$((errors + 1))
  fi

  # Compare source PNG hash against artifact PNG hash to catch visual drift.
  # This catches background-image / texture changes that dimension checks miss.
  source_hash="$(sha256sum "${source_path}" | awk '{print $1}')"
  artifact_hash="$(sha256sum "${artifact_path}" | awk '{print $1}')"
  if [[ "${source_hash}" != "${artifact_hash}" ]]; then
    echo "error: visual drift for ${target_id}: source hash=${source_hash} artifact hash=${artifact_hash}" >&2
    errors=$((errors + 1))
  fi

    echo "ok: ${target_id} (${tier})"
  done < <(jq -r '.targets[] | [.id, .source_generated_png, .tier] | @tsv' "${MANIFEST_PATH}")

  while IFS= read -r artifact_file; do
    artifact_base="$(basename "${artifact_file}")"
    artifact_id="${artifact_base%.png}"
    if ! jq -e --arg id "${artifact_id}" '.targets | any(.id == $id)' "${MANIFEST_PATH}" >/dev/null 2>&1; then
      echo "error: undeclared artifact produced in freeze scope: ${artifact_file}" >&2
      errors=$((errors + 1))
    fi
  done < <(find "${ARTIFACT_DIR}" -maxdepth 1 -type f -name '*.png' | sort)
else
  checked="$(jq '.targets | length' "${MANIFEST_PATH}")"
fi

if [[ "${VALIDATION_MODE}" == "full" ]]; then
  if ! jq -e '.schema_version == 1 and (.artifacts | type == "array")' "${FREEZE_FILE}" >/dev/null 2>&1; then
    echo "error: invalid freeze file schema: ${FREEZE_FILE}" >&2
    errors=$((errors + 1))
  else
    manifest_ids_file="$(mktemp)"
    freeze_ids_file="$(mktemp)"

    jq -r '.targets[].id' "${MANIFEST_PATH}" | sort > "${manifest_ids_file}"
    jq -r '.artifacts[].id' "${FREEZE_FILE}" | sort > "${freeze_ids_file}"

    if ! diff -u "${manifest_ids_file}" "${freeze_ids_file}" >/dev/null 2>&1; then
      echo "error: freeze file target ids do not match manifest ids: ${FREEZE_FILE}" >&2
      errors=$((errors + 1))
    fi

      while IFS=$'\t' read -r id artifact_rel frozen_hash frozen_w frozen_h frozen_channels; do
      [[ -n "${id}" ]] || continue
        if [[ "${VALIDATION_MODE}" != "full" ]]; then
          continue
        fi
      if [[ "${artifact_rel}" = /* ]]; then
        frozen_artifact_path="${artifact_rel}"
      else
        frozen_artifact_path="${REPO_ROOT}/${artifact_rel}"
      fi

      if [[ ! -f "${frozen_artifact_path}" ]]; then
        echo "error: freeze artifact missing for ${id}: ${frozen_artifact_path}" >&2
        errors=$((errors + 1))
        continue
      fi

      actual_hash="$(sha256sum "${frozen_artifact_path}" | awk '{print $1}')"
      actual_meta="$(magick identify -format '%w %h %[channels]' "${frozen_artifact_path}" 2>/dev/null || true)"
      if [[ -z "${actual_meta}" ]]; then
        echo "error: freeze artifact inspect failed for ${id}: ${frozen_artifact_path}" >&2
        errors=$((errors + 1))
        continue
      fi
      read -r actual_w actual_h actual_channels <<< "${actual_meta}"
      actual_channels="$(echo "${actual_channels}" | awk '{print tolower($1)}')"
      frozen_channels="$(echo "${frozen_channels}" | awk '{print tolower($1)}')"

      if [[ "${actual_hash}" != "${frozen_hash}" ]]; then
        echo "error: freeze hash mismatch for ${id}: expected=${frozen_hash} actual=${actual_hash}" >&2
        errors=$((errors + 1))
      fi
      if [[ "${actual_w}" != "${frozen_w}" || "${actual_h}" != "${frozen_h}" ]]; then
        echo "error: freeze dimension mismatch for ${id}: expected=${frozen_w}x${frozen_h} actual=${actual_w}x${actual_h}" >&2
        errors=$((errors + 1))
      fi
      if [[ "${actual_channels}" != "${frozen_channels}" ]]; then
        echo "error: freeze channel mismatch for ${id}: expected=${frozen_channels} actual=${actual_channels}" >&2
        errors=$((errors + 1))
      fi
    done < <(jq -r '.artifacts[] | [.id, .artifact_path, .sha256, (.width|tostring), (.height|tostring), .channels] | @tsv' "${FREEZE_FILE}")

    rm -f "${manifest_ids_file}" "${freeze_ids_file}"
  fi
fi

if [[ "${checked}" -eq 0 ]]; then
  echo "error: no targets found in manifest: ${MANIFEST_PATH}" >&2
  exit 1
fi

if [[ "${errors}" -gt 0 ]]; then
  echo "validation failed: ${errors} issue(s) across ${checked} target(s)" >&2
  exit 1
fi

echo "validation passed: ${checked} target(s) in ${MANIFEST_PATH}"
