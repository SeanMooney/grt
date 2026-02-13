# Gertty Search Language

**Source project:** gertty
**Source files:** `gertty/search/__init__.py`, `gertty/search/tokenizer.py`, `gertty/search/parser.py`
**Status:** Draft
**Informs:** `search-engine.md`

## Overview

Gertty implements a local search language that mirrors the Gerrit web UI's search syntax. Because gertty operates offline against a local SQLite database (via SQLAlchemy), it cannot simply forward search queries to the Gerrit REST API. Instead, it includes a full lexer/parser pipeline that translates Gerrit-style query strings into SQLAlchemy filter expressions, which are then applied to the local database.

The search module has three components:

1. **`tokenizer.py`** -- Uses PLY (Python Lex-Yacc) `ply.lex` to break a query string into tokens.
2. **`parser.py`** -- Uses PLY `ply.yacc` to define a grammar over those tokens, where each production rule directly constructs a SQLAlchemy expression object.
3. **`__init__.py`** -- Contains the `SearchCompiler` class that orchestrates lexing and parsing, performs post-parse table join resolution, and exposes the final query expression to the rest of the application.

The result is that a query string like `status:open owner:self project:myproject` is parsed into a SQLAlchemy `and_()` expression that can be passed directly to a `SELECT` statement's `WHERE` clause. The parser does not produce an intermediate AST -- it constructs executable query fragments inline during parsing, a design choice that couples the grammar tightly to the database schema but keeps the implementation compact.

Gertty also adds several extension operators not present in Gerrit's search language: `recentlyseen:` (filter by when changes were last viewed locally) and `is:held` (a local hold flag for changes the user wants to defer).

## Tokenizer

The tokenizer is built with `ply.lex` and is constructed via the `SearchTokenizer()` factory function. It defines the following token types:

### Operator Tokens

A dictionary maps operator name strings to token type names. When the lexer encounters a pattern matching `[a-zA-Z_][a-zA-Z_]*:` (a word followed by a colon), it looks up the prefix in this dictionary. If found, it emits the corresponding operator token (e.g., `OP_STATUS`, `OP_OWNER`). If not found, it emits a generic `OP` token, which the parser treats as a syntax error.

The full operator map:

| Operator Name   | Token Type         | Notes                       |
|-----------------|--------------------|-----------------------------|
| `age`           | `OP_AGE`           |                             |
| `recentlyseen`  | `OP_RECENTLYSEEN`  | Gertty extension            |
| `change`        | `OP_CHANGE`        |                             |
| `owner`         | `OP_OWNER`         |                             |
| `reviewer`      | `OP_REVIEWER`      |                             |
| `commit`        | `OP_COMMIT`        |                             |
| `project`       | `OP_PROJECT`       |                             |
| `projects`      | `OP_PROJECTS`      | Prefix-match variant        |
| `_project_key`  | `OP_PROJECT_KEY`   | Internal use only           |
| `branch`        | `OP_BRANCH`        |                             |
| `topic`         | `OP_TOPIC`         |                             |
| `hashtag`       | `OP_HASHTAG`       |                             |
| `ref`           | `OP_REF`           |                             |
| `label`         | `OP_LABEL`         |                             |
| `message`       | `OP_MESSAGE`       |                             |
| `comment`       | `OP_COMMENT`       |                             |
| `file`          | `OP_FILE`          |                             |
| `path`          | `OP_PATH`          |                             |
| `has`           | `OP_HAS`           |                             |
| `is`            | `OP_IS`            |                             |
| `status`        | `OP_STATUS`        |                             |
| `limit`         | `OP_LIMIT`         |                             |

Several operators that would require local group membership data (`ownerin`, `reviewerin`) or tracking IDs (`tr`, `bug`) are commented out in the source.

### Literal and String Tokens

- **`NUMBER`** -- Matches `\d+` and converts the value to a Python `int`.
- **`INTEGER`** -- Matches `[+-]\d+` (signed integer) and converts to `int`. Used for label values like `+2` or `-1`.
- **`CHANGE_ID`** -- Matches the pattern `I[a-fA-F0-9]{7,40}`, which is the Gerrit Change-Id format (the letter `I` followed by 7-40 hex characters).
- **`SSTRING`** -- Single-quoted string. The lexer strips the quotes and handles `\'` and `\\` escape sequences.
- **`DSTRING`** -- Double-quoted string. Same escape handling as `SSTRING` but with `\"`.
- **`USTRING`** -- Unquoted string. Matches `[^\s\(\)!-][^\s\(\)!]*` -- any sequence of non-whitespace characters that does not start with `(`, `)`, `!`, or `-`, and does not contain those delimiters. This is the fallback token for bare words like `open`, `self`, or `myproject`.

### Boolean and Structural Tokens

- **`AND`** -- Matches `and` or `AND` (case-sensitive to these two forms).
- **`OR`** -- Matches `or` or `OR`.
- **`NOT`** -- Matches `not` or `NOT`.
- **`NEG`** -- Matches `-` or `!` as prefix negation operators.
- **`LPAREN`** / **`RPAREN`** -- Parentheses for grouping.

### Whitespace and Error Handling

- Spaces and tabs are ignored (`t_ignore = ' \t'`).
- Newlines increment the line counter but are otherwise ignored.
- Illegal characters print an error and are skipped.

### Token Precedence and Ordering

PLY resolves token ambiguity by function definition order and string length. In the gertty tokenizer:

1. `t_OP` is a function rule, so it has priority over string rules for matching `word:` patterns.
2. `t_AND`, `t_OR`, `t_NOT` are function rules that match before `t_USTRING`, ensuring that `and`, `or`, and `not` are recognized as boolean keywords rather than bare strings.
3. `t_USTRING` is the lowest-priority catch-all for any non-special text.

## Parser Grammar

The parser is built with `ply.yacc` and constructed via the `SearchParser()` factory function. It defines a context-free grammar where each production rule directly constructs a SQLAlchemy expression.

### Precedence Declaration

```python
precedence = (
    ('left', 'NOT', 'NEG'),
)
```

Only `NOT` and `NEG` have explicit precedence, declared as left-associative. All other operators rely on the grammar structure for precedence.

### Grammar Rules

The top-level production is `expression`, which can be any of the following:

```
expression : list_expr
           | paren_expr
           | boolean_expr
           | negative_expr
           | term
```

**Implicit AND (`list_expr`)**:

```
list_expr : expression expression
```

Two adjacent expressions with no explicit boolean operator are combined with `and_()`. This mirrors Gerrit's behavior where `status:open owner:self` means `status:open AND owner:self`.

**Parenthetical grouping (`paren_expr`)**:

```
paren_expr : LPAREN expression RPAREN
```

Parentheses override the default precedence, passing through the inner expression unchanged.

**Boolean expressions (`boolean_expr`)**:

```
boolean_expr : expression AND expression
             | expression OR expression
```

Explicit `AND` or `OR` keywords combine two expressions using `and_()` or `or_()` respectively. The keyword comparison is case-insensitive (via `.lower()`).

**Negation (`negative_expr`)**:

```
negative_expr : NOT expression
              | NEG expression
```

Both `NOT`/`not` and the `-`/`!` prefix operators produce a `not_()` wrapper around the inner expression.

**Terms**:

```
term : age_term | recentlyseen_term | change_term | owner_term
     | reviewer_term | commit_term | project_term | projects_term
     | project_key_term | branch_term | topic_term | hashtag_term
     | ref_term | label_term | message_term | comment_term
     | has_term | is_term | status_term | file_term | path_term
     | limit_term | op_term
```

Each term maps to a specific operator handler. The `op_term` rule catches any unrecognized `OP` token and raises a `SyntaxError`.

**String normalization**:

```
string : SSTRING | DSTRING | USTRING
```

All three string types are unified into a single `string` non-terminal so that operator rules can accept any quoting style.

### AST Structure (or Lack Thereof)

Gertty does not produce an abstract syntax tree. The parser's semantic actions directly construct SQLAlchemy `ClauseElement` objects. The "tree" that emerges is a SQLAlchemy expression tree (nested `and_()`, `or_()`, `not_()`, comparison, and subquery nodes). This means the output of `parser.parse()` is a single SQLAlchemy expression ready for use in a `WHERE` clause. The advantage is simplicity -- there is no separate evaluation step. The disadvantage is that the parser is inseparable from the database schema.

## Query Operators

### Field Operators

Each field operator maps a `field:value` search term to a SQLAlchemy expression against the local database. The patterns fall into several categories:

**Direct column comparison** -- the simplest pattern, comparing a column value directly:

- **`status:VALUE`** -- Compares `change_table.c.status`. Recognizes the synthetic values `open` (status not in `MERGED`, `ABANDONED`) and `closed` (status in `MERGED`, `ABANDONED`). All other values are uppercased and compared directly.
- **`branch:VALUE`** -- Compares `change_table.c.branch`. Supports regex if the value starts with `^` (via `func.matches()`).
- **`topic:VALUE`** -- Compares `change_table.c.topic`. Regex-capable. For exact matches, also asserts that topic `IS NOT NULL`.
- **`project:VALUE`** -- Compares `project_table.c.name`. Regex-capable.
- **`projects:VALUE`** -- Uses SQL `LIKE` with a trailing wildcard (`value%`) for prefix matching across project names.
- **`_project_key:NUMBER`** -- Internal operator comparing `change_table.c.project_key` directly by numeric key. Not exposed to users.
- **`ref:VALUE`** -- Compares against `change_table.c.branch` with the `refs/heads/` prefix stripped (or applied for regex matching).
- **`hashtag:VALUE`** -- Compares `hashtag_table.c.name`. Regex-capable.

**Owner and account resolution**:

- **`owner:VALUE`** -- If the value is `self`, resolves to the current user's account ID (stored on `p.parser.account_id`). Otherwise, matches against `account_table.c.username`, `.email`, or `.name` using `or_()`.

**Subquery-based operators** -- these operators search across related tables by constructing a subselect that returns matching `change.key` values:

- **`reviewer:VALUE`** -- Joins `approval_table` and `account_table` to find changes where a specific reviewer has left an approval. Supports `self`, numeric account IDs, or username/email/name matching. Produces `change_table.c.key.in_(subselect)`.
- **`commit:VALUE`** -- Joins `revision_table` to find changes containing a specific commit hash.
- **`message:VALUE`** -- Joins `revision_table` and uses `LIKE '%%value%%'` to search commit messages.
- **`comment:VALUE`** -- Searches both revision messages (exact match) and inline comments (`comment_table.c.message`). Returns changes matching either, combined with `or_()`.

**File operators**:

- **`file:VALUE`** -- Searches `file_table.c.path` and `.old_path`. If the value starts with `^`, it is treated as a regex. Otherwise, the value is escaped and wrapped in a regex pattern `(^|.*/)VALUE(/.*|$)` to match the filename anywhere in the path hierarchy. Also filters for non-null file status.
- **`path:VALUE`** -- Like `file:` but without the path wrapping -- performs an exact match on the full path (or regex if prefixed with `^`).

**Label operator** -- the most complex field operator:

- **`label:LABEL_EXPRESSION`** -- Parses a compound expression using a regex: `label_name[operator]value[,user=username]`. Examples: `label:Code-Review>=+1`, `label:Verified=+1,user=self`. The operator can be `=`, `>=`, or `<=`. The parser joins `approval_table` (and optionally `account_table` for user filtering) and produces a subselect.

The label regex pattern is:
```python
r'(?P<label>[a-zA-Z0-9_-]+([a-zA-Z]|((?<![-+])[0-9])))'
r'(?P<operator>[<>]?=?)(?P<value>[-+]?[0-9]+)'
r'($|,(user=)?(?P<user>\S+))'
```

**Age operators**:

- **`age:NUMBER UNIT`** -- Compares `change_table.c.updated` against a calculated timestamp. The grammar is `OP_AGE NUMBER string`, where the number and unit string (e.g., `2 days`) are combined. Supported time units: `seconds`/`sec`/`s`, `minutes`/`min`/`m`, `hours`/`hour`/`hr`/`h`, `days`/`day`/`d`, `weeks`/`week`/`w`, `months`/`month`/`mon`, `years`/`year`/`y`. The comparison is `updated < (now - delta)`, meaning "changes older than the given age."
- **`recentlyseen:NUMBER UNIT`** -- Gertty extension. Uses `func.max(change_table.c.last_seen)` minus the given delta as a correlated subselect threshold, finding changes seen within the given period relative to the most recently seen change. This enables queries like `recentlyseen:24 hours`.

### Boolean Operators

- **`AND`** / **`and`** -- Explicit conjunction. Produces `and_(left, right)`.
- **`OR`** / **`or`** -- Disjunction. Produces `or_(left, right)`.
- **`NOT`** / **`not`** -- Prefix negation. Produces `not_(expression)`.
- **`-`** / **`!`** -- Prefix negation (shorthand). Same behavior as `NOT`.
- **Implicit AND** -- Two adjacent terms with no explicit operator are combined with `and_()`. This is the most common form in practice: `status:open owner:self` is equivalent to `status:open AND owner:self`.
- **Parenthetical grouping** -- `(expression)` overrides default associativity.

The precedence structure means that in `a OR b c`, the implicit AND between `b` and `c` binds more tightly than `OR`, producing `a OR (b AND c)`. This matches Gerrit's behavior.

### Special Operators

**`is:` operator** -- Tests boolean flags and status shortcuts:

| Value       | SQL Expression                                    | Notes              |
|-------------|---------------------------------------------------|---------------------|
| `reviewed`  | Subselect: approvals with non-zero value exist    |                     |
| `open`      | `status NOT IN ('MERGED', 'ABANDONED')`           |                     |
| `closed`    | `status IN ('MERGED', 'ABANDONED')`               |                     |
| `submitted` | `status == 'SUBMITTED'`                           |                     |
| `merged`    | `status == 'MERGED'`                              |                     |
| `abandoned` | `status == 'ABANDONED'`                           |                     |
| `owner`     | `account_table.c.id == account_id`                | Current user owns   |
| `starred`   | `change_table.c.starred == True`                  |                     |
| `held`      | `change_table.c.held == True`                     | Gertty extension    |
| `reviewer`  | Subselect: current user has approvals on change   |                     |
| `watched`   | `project_table.c.subscribed == True`              |                     |

Unsupported `is:` values raise `SearchSyntaxError`.

**`has:` operator** -- Currently only supports `has:draft`:

- **`has:draft`** -- Joins `revision_table` and `message_table` to find changes that have draft messages (`message_table.c.draft == True`). Produces a subselect.

Other `has:` values raise `SearchSyntaxError`. A TODO comment notes that `has:star` is not yet implemented.

**`limit:` operator** -- Declared in the grammar but implemented as a no-op:

```python
def p_limit_term(p):
    '''limit_term : OP_LIMIT NUMBER'''
    p[0] = (True == True)
```

A comment explains that `limit` cannot be expressed as a SQLAlchemy `WHERE` clause filter -- it would need to be applied to the query object itself (via `.limit()`), which requires an out-of-band mechanism the current architecture does not support. The no-op ensures that queries containing `limit:` do not produce syntax errors.

**`change:` operator** -- Looks up a specific change by number or Change-Id:

```python
def p_change_term(p):
    '''change_term : OP_CHANGE CHANGE_ID
                   | OP_CHANGE NUMBER'''
    if type(p[2]) == int:
        p[0] = gertty.db.change_table.c.number == p[2]
    else:
        p[0] = gertty.db.change_table.c.change_id == p[2]
```

If the argument is a number, it matches by change number. If it matches the `I[a-fA-F0-9]{7,40}` pattern, it matches by Change-Id.

## Query Execution

The `SearchCompiler` class in `__init__.py` orchestrates the full pipeline from query string to executable SQLAlchemy expression.

### Compilation Pipeline

1. **Instantiation** -- `SearchCompiler.__init__()` creates a `SearchTokenizer` (lexer) and `SearchParser` (parser). It also accepts a `get_account_id` callback for resolving the `self` keyword, and stores `account_id` on the parser object so that production rules can access it via `p.parser.account_id`.

2. **Parsing** -- `SearchCompiler.parse(data)` calls `self.parser.parse(data, lexer=self.lexer)`, which runs the PLY lexer and parser together. The result is a single SQLAlchemy expression.

3. **Table discovery** -- `SearchCompiler.findTables(expression)` walks the SQLAlchemy expression tree using a stack-based traversal. For each node that has a `.table` attribute (i.e., column references), it collects which tables are referenced, skipping `change_table` (which is always the primary query target) and any subselects (which handle their own joins internally).

4. **Join injection** -- After `findTables()` identifies which auxiliary tables appear in the expression, the compiler wraps the expression with the necessary join conditions:

```python
if gertty.db.project_table in tables:
    result = and_(change_table.c.project_key == project_table.c.key, result)

if gertty.db.account_table in tables:
    result = and_(change_table.c.account_key == account_table.c.key, result)

if gertty.db.hashtag_table in tables:
    result = and_(hashtag_table.c.change_key == change_table.c.key, result)

if gertty.db.file_table in tables:
    # Restrict to the most recent revision
    s = select([func.max(revision_table.c.number)], correlate=False).where(
        revision_table.c.change_key == change_table.c.key
    ).correlate(change_table)
    result = and_(
        file_table.c.revision_key == revision_table.c.key,
        revision_table.c.change_key == change_table.c.key,
        revision_table.c.number == s,
        result
    )
```

The file table join is the most complex: it adds a correlated subquery to ensure only files from the latest revision are matched, not files from older patchsets.

5. **Error handling** -- If any tables remain unresolved after the known join patterns, the compiler raises an exception. The `SearchSyntaxError` exception class provides error messages with position information (column offset within the query string).

### Key Design Observations

- **No intermediate representation**: The parser produces SQLAlchemy expressions directly. There is no intermediate AST, query plan, or optimization pass.
- **Subselects for cross-table queries**: Operators that need data from related tables (reviewer, commit, message, comment, has:draft, is:reviewed) construct their own subselects internally. This means those joins are self-contained and do not require post-parse table discovery.
- **Top-level join injection**: Only direct column references to auxiliary tables (project, account, hashtag, file) trigger post-parse join injection. This two-tier approach keeps the grammar rules simpler.
- **Regex support**: Several operators support regex matching when the value starts with `^`, delegating to a custom `func.matches()` SQLAlchemy function (presumably registered as a SQLite user-defined function).
- **Account resolution**: The `self` keyword is resolved eagerly at parse time using the `account_id` stored on the parser. This is fetched lazily on first use via the `get_account_id` callback.

## grt Divergences

The following areas represent concrete differences between gertty's Python/PLY/SQLAlchemy approach and grt's planned Rust implementation:

**Parser technology**: Gertty uses PLY (Python Lex-Yacc), a runtime parser generator that constructs lexer and parser tables dynamically. grt will use a Rust parsing library such as `pest` (PEG grammar, compile-time code generation), `nom` (parser combinator library), or `winnow` (a nom successor). Any of these will provide compile-time type safety that PLY cannot. A PEG grammar with `pest` would allow the grammar to be defined declaratively in a `.pest` file, while `nom`/`winnow` would allow inline combinator-style parsing. The grammar is simple enough that either approach is viable.

**AST as intermediate representation**: Gertty skips the AST and builds SQLAlchemy expressions directly in parser actions. grt should define an explicit AST enum (e.g., `SearchExpr::And(Box<SearchExpr>, Box<SearchExpr>)`, `SearchExpr::Field { op: FieldOp, value: String }`) and separate parsing from query planning. This enables:
  - Query validation independent of the database
  - Potential query optimization passes (e.g., reordering filters by selectivity)
  - Reuse of the AST for display, serialization, or fuzzy matching integration
  - Unit testing of the parser without a database

**SQLite integration**: Gertty uses SQLAlchemy's expression language to build queries that are later compiled to SQL. grt will use `rusqlite` with raw SQL or a query builder. The subselect patterns gertty uses (for reviewer, commit, message, comment, label, and has:draft queries) will need to be translated to SQL string construction or a Rust query builder. Given that `rusqlite` does not have SQLAlchemy's expression composition, grt may benefit from a lightweight query builder or macro-based SQL construction.

**Fuzzy matching with nucleo-matcher**: Gertty only supports exact matching and regex (`^`-prefixed values). grt can integrate `nucleo-matcher` to provide fuzzy matching for interactive use (TUI search-as-you-type), while preserving exact and regex modes for scripted/CLI use. This would require the AST to distinguish between match modes (exact, regex, fuzzy) so the query executor can dispatch accordingly.

**Compiled query plans**: Because gertty builds SQLAlchemy expressions at parse time, every parse produces a new expression tree. grt could cache compiled query plans (prepared statements with parameter bindings) for frequently-used query patterns, improving performance for repeated searches. The explicit AST enables hashing/equality comparison to detect equivalent queries.

**Type safety for operators**: Gertty's operator handling is stringly-typed -- operator names are looked up in a dictionary, and `is:`/`has:` values are compared as raw strings with fallthrough to error. grt can use Rust enums for operator types and match exhaustively, ensuring at compile time that all operators are handled.

**Regex handling**: Gertty delegates regex matching to a `func.matches()` function, presumably a SQLite UDF. grt can register a similar UDF with `rusqlite` using the `regex` crate, which provides significantly better regex performance than Python's `re` module. Alternatively, grt could compile regexes once and apply them in Rust-side filtering for operations that cannot be pushed down to SQLite.

**`limit:` implementation**: Gertty's `limit:` is a no-op because the architecture cannot thread it back to the query. grt's explicit AST can represent `limit` as a top-level query modifier rather than a filter predicate, allowing the query executor to apply `.limit()` to the final SQL query.

**Error reporting**: Gertty's error messages include column offsets but no structured error types. grt can use `miette` or `ariadne` for rich diagnostic output with source spans, colored underlines, and suggestions -- consistent with modern Rust CLI tooling.

**Extension operators**: Gertty adds `recentlyseen:` and `is:held` as local extensions. grt should establish a clear convention for distinguishing standard Gerrit operators from grt-specific extensions, potentially using a namespace or documentation convention to avoid confusion.
