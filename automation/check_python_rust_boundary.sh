#!/usr/bin/env bash
# =============================================================================
# check_python_rust_boundary.sh
# Граничные проверки Rust-миграции R5–R8.
# Запускать после каждого изменения в app/ или app/rust/.
# Возвращает код 1 при первом нарушении.
# =============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VIOLATIONS=0

report_violation() {
    echo "❌ НАРУШЕНИЕ: $1" >&2
    VIOLATIONS=$((VIOLATIONS + 1))
}

# ---------------------------------------------------------------------------
# R0: в runtime app/ не должно быть Python-файлов
# ---------------------------------------------------------------------------
if find app -type f -name "*.py" | grep -q .; then
  report_violation "В app/ обнаружены .py файлы — runtime должен быть Rust-only."
fi

# ---------------------------------------------------------------------------
# R5: embed/ не должен импортировать voyageai или qdrant_client напрямую
# ---------------------------------------------------------------------------
if rg -rn "import voyageai|from voyageai|from qdrant_client|import qdrant_client" \
    infra/analytics_lab/src/ --glob "*.py" 2>/dev/null | grep -q .; then
  report_violation "Python runtime-path не должен использовать voyageai/qdrant_client напрямую."
fi

# ---------------------------------------------------------------------------
# R5: voyageai и qdrant-client не должны быть в корневом requirements.txt
# ---------------------------------------------------------------------------
if grep -qE "^voyageai|^qdrant-client" requirements.txt 2>/dev/null; then
  report_violation "requirements.txt содержит voyageai или qdrant-client — эти зависимости перенесены в Rust (R5)."
fi

# ---------------------------------------------------------------------------
# R6: psycopg не должен импортироваться вне analytics_svc
# ---------------------------------------------------------------------------
if rg -rn "import psycopg|from psycopg" \
    automation/ --glob "*.py" 2>/dev/null | grep -q .; then
  report_violation "psycopg используется в automation — SQL-доступ должен идти через Rust/sqlx (R6)."
fi

# ---------------------------------------------------------------------------
# R6: psycopg не должен быть в корневом requirements.txt
# ---------------------------------------------------------------------------
if grep -qE "^psycopg" requirements.txt 2>/dev/null; then
  report_violation "requirements.txt содержит psycopg — зависимость перенесена в Rust/sqlx (R6)."
fi

# ---------------------------------------------------------------------------
# R7: в Python runtime не допускается локальный рендеринг/планирование контента
# ---------------------------------------------------------------------------
if rg -n "def render_|def generate_block|def plan_blocks" \
    infra/analytics_lab/src/ --glob "*.py" 2>/dev/null | grep -q .; then
  report_violation "Python runtime-path содержит локальный рендеринг/планировщик — перенос в Rust обязателен."
fi

# ---------------------------------------------------------------------------
# R8: graph/ не должен импортироваться из Python runtime-path
# ---------------------------------------------------------------------------
if rg -rn "from app\\.graph|import app\\.graph|from pipeline\\.graph|import pipeline\\.graph" \
    infra/analytics_lab/src/ --glob "*.py" 2>/dev/null | grep -q .; then
  report_violation "graph/ логика не должна импортироваться из Python runtime-path (R8)."
fi

# ---------------------------------------------------------------------------
# R8: neo4j не должен быть в корневом requirements.txt (только analytics_lab)
# ---------------------------------------------------------------------------
if grep -qE "^neo4j" requirements.txt 2>/dev/null; then
  report_violation "requirements.txt содержит neo4j — перенесён в infra/analytics_lab/requirements.txt (R8)."
fi

# ---------------------------------------------------------------------------
# Итог
# ---------------------------------------------------------------------------
if [ "$VIOLATIONS" -eq 0 ]; then
    echo "✅ Граничные проверки R5–R8: все нарушения отсутствуют."
    exit 0
else
    echo "Итого нарушений: $VIOLATIONS" >&2
    exit 1
fi
