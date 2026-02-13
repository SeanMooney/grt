# Gertty Search Language

**Source project:** gertty
**Source files:** `gertty/search/`, `gertty/search/tokenizer.py`, `gertty/search/parser.py`
**Status:** Stub
**Informs:** `search-engine.md`

## Overview

<!-- TODO: Gertty's Gerrit-compatible search query language -->

## Tokenizer

<!-- TODO: Token types, lexing rules, special characters -->

## Parser Grammar

<!-- TODO: Grammar rules, operator precedence, AST structure -->

## Query Operators

<!-- TODO: Supported operators and their SQL mapping -->

### Field Operators
<!-- status:, owner:, project:, branch:, topic:, label:, etc. -->

### Boolean Operators
<!-- AND, OR, NOT, parenthetical grouping -->

### Special Operators
<!-- is:starred, is:reviewed, has:draft, age:, limit: -->

## Query Execution

<!-- TODO: How parsed queries map to SQLite queries -->

## grt Divergences

<!-- TODO: Where grt's search will differ:
- Integration with nucleo-matcher for fuzzy search
- Potential for compiled query plans
- Rust parser (pest/nom) vs Python tokenizer
-->
