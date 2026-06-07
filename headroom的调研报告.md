# Headroom 的会话相关压缩设计调研报告

> 调研对象：`reference-projects/headroom-main`
>
> 当前日期：2026-06-08
>
> 目标：把 Headroom 中所有与“会话上下文、压缩、缓存、恢复、跨 agent 记忆/上下文传递”相关的设计梳理清楚，并明确哪些能学习到 memorph，哪些不能照搬。

---

## 0. 结论先行

Headroom 对 memorph 最有价值的不是某一个压缩算法，而是一套压缩工程纪律：

1. **压缩不是删除历史，而是把大内容变成可恢复的短表示。**
   Headroom 的 CCR（Compress-Cache-Retrieve）核心思想是：线上 prompt 可以是压缩后的，但原文必须能通过 hash/store/tool 再取回。memorph 应该对应为：canonical compressed block + durable archive + `memorph-archive://...` + query-first retrieve。

2. **最新架构方向明确否定“整段历史打分后丢弃”的旧模型。**
   Headroom `REALIGNMENT/00-overview.md` 明确指出旧模型 `IntelligentContextManager`/RollingWindow/MessageScorer 是错误抽象，会破坏 provider prompt cache。新的正确模型是：**passthrough sacred，cache hot zone 不动，只压 live zone 的大块内容**。

3. **Headroom 的 runtime request compression 与 memorph 的 session migration compression 不是同一层。**
   Headroom 主要在请求发给 LLM 前压缩 live-zone blocks。memorph 要解决的是跨 agent 会话中间态、会话文件、provider-native/portable compression marker 的转换与恢复。因此不能照搬 Headroom 的短 TTL cache，但可以学习它的 CCR 语义、manifest、gate、type-aware routing、hot-zone 保护。

4. **Headroom 自己的文档存在“旧文档 vs 新 realignment”的冲突。**
   `docs/content/docs/context-management.mdx` 仍描述 IntelligentContext 会按消息打分、丢低分消息、存 CCR；但 `docs/content/docs/ccr.mdx` 又写明 Message-level CCR via IntelligentContext 已 retired。`REALIGNMENT/00-overview.md` 和 Rust live-zone 代码更可信，应该作为学习主线。

5. **对 memorph 来说，最该学习的是这 8 个点：**
   - CCR：原文持久归档，压缩块只带摘要和 archive ref。
   - query-first retrieval：恢复先按 query 局部检索，不默认 full expand。
   - live-zone/hot-zone：最近任务状态、系统指令、工具定义、provider 内部块不压。
   - content routing：日志、搜索结果、diff、JSON、代码、plain text 分策略。
   - token/size validation gate：压缩后不小就拒绝，不能为了压而压。
   - manifest/outcome：每个候选都要记录为什么压、为什么跳过、为什么拒绝。
   - byte/cache safety：跨 provider 投影时只替换压缩块，不扰动无关上下文。
   - retrieval metadata：返回结果要说明是 full 还是 partial，返回了哪些 event，遗漏了多少。

---

## 1. 本次调研读取的关键文件

### 1.1 顶层说明和产品定位

- `reference-projects/headroom-main/README.md`
- `reference-projects/headroom-main/docs/content/docs/architecture.mdx`
- `reference-projects/headroom-main/docs/content/docs/how-compression-works.mdx`
- `reference-projects/headroom-main/docs/content/docs/ccr.mdx`
- `reference-projects/headroom-main/docs/content/docs/context-management.mdx`
- `reference-projects/headroom-main/docs/content/docs/memory.mdx`
- `reference-projects/headroom-main/docs/content/docs/shared-context.mdx`

这些文件说明 Headroom 的产品层能力：proxy、SDK、MCP、CCR、memory、shared context、content router、cache optimization。

### 1.2 架构纠偏文档

- `reference-projects/headroom-main/REALIGNMENT/00-overview.md`
- `reference-projects/headroom-main/REALIGNMENT/02-architecture.md`
- `reference-projects/headroom-main/REALIGNMENT/04-phase-B-live-zone.md`
- `reference-projects/headroom-main/REALIGNMENT/12-decisions-needed.md`

这些文件比普通 docs 更关键，因为它们说明 Headroom 团队自己后续如何修正旧设计。尤其是：

- 不再把“压缩”理解成“从 messages array 里删历史”。
- 不再把 ICM/RollingWindow/MessageScorer 当核心。
- 转向 live-zone-only compression。
- 强调 cache hot zone 不能被改。
- 强调 request byte fidelity。

### 1.3 Rust 核心实现

- `reference-projects/headroom-main/crates/headroom-core/src/ccr/mod.rs`
- `reference-projects/headroom-main/crates/headroom-core/src/ccr/backends/*`
- `reference-projects/headroom-main/crates/headroom-core/src/transforms/live_zone.rs`
- `reference-projects/headroom-main/crates/headroom-core/src/transforms/content_detector.rs`
- `reference-projects/headroom-main/crates/headroom-core/src/transforms/log_compressor.rs`
- `reference-projects/headroom-main/crates/headroom-core/src/transforms/search_compressor.rs`
- `reference-projects/headroom-main/crates/headroom-core/src/transforms/diff_compressor.rs`
- `reference-projects/headroom-main/crates/headroom-core/src/transforms/adaptive_sizer.rs`
- `reference-projects/headroom-main/crates/headroom-core/src/tokenizer/*`

这些文件是实际实现的压缩纪律：CCR store、BLAKE3 hash、marker、live-zone dispatcher、token validation、manifest、content detection、per-type compressor。

### 1.4 Python runtime 和会话记忆相关实现

- `reference-projects/headroom-main/headroom/cache/compression_store.py`
- `reference-projects/headroom-main/headroom/memory/models.py`
- `reference-projects/headroom-main/headroom/memory/core.py`
- `reference-projects/headroom-main/headroom/proxy/memory_handler.py`
- `reference-projects/headroom-main/headroom/proxy/memory_injection.py`
- `reference-projects/headroom-main/headroom/memory/storage_router.py`
- `reference-projects/headroom-main/headroom/memory/writers/*`
- `reference-projects/headroom-main/headroom/memory/sync_adapters/*`

这些文件体现“会话之外的长期记忆”和“跨 agent 记忆同步”的设计。它们不是 session compression 的直接实现，但对 memorph 的 workspace/session/agent scoped memory 有参考价值。

### 1.5 测试证据

- `reference-projects/headroom-main/crates/headroom-core/tests/ccr_roundtrip.rs`
- `reference-projects/headroom-main/crates/headroom-core/tests/live_zone_ccr.rs`
- `reference-projects/headroom-main/crates/headroom-core/tests/live_zone_dispatch.rs`
- `reference-projects/headroom-main/crates/headroom-core/tests/live_zone_thresholds.rs`
- `reference-projects/headroom-main/crates/headroom-core/tests/live_zone_token_validation.rs`
- `reference-projects/headroom-main/tests/test_ccr_response_handler.py`
- `reference-projects/headroom-main/tests/test_ccr_tool_injection.py`
- `reference-projects/headroom-main/tests/test_compression_store.py`
- `reference-projects/headroom-main/tests/test_memory_handler_project_isolation.py`
- `reference-projects/headroom-main/tests/test_memory_injection_budget.py`

这些测试说明哪些行为是 Headroom 认为必须锁住的契约。

---

## 2. Headroom 的整体定位

### 2.1 Headroom 做的事情

Headroom 是一个“LLM 请求前的上下文压缩层”。它可以通过 proxy、SDK、middleware 或工具集成接入，在请求到达 Anthropic/OpenAI/Gemini/Bedrock 等 provider 前处理上下文。

它压缩的对象包括：

- tool outputs。
- logs/build/test output。
- search/ripgrep results。
- RAG chunks。
- files。
- conversation history。
- agent handoff context。
- memory snippets。

但是从新架构看，最安全的生产方向不是“随便动整段 history”，而是“只压 live-zone 中的大块内容”。

### 2.2 Headroom 和 memorph 的关键区别

Headroom 是 **runtime request layer**：

```text
client / agent / app
-> Headroom proxy 或 SDK
-> 压缩当前请求中的 live-zone 内容
-> 发给 LLM provider
-> response 原样返回或由 CCR handler 自动处理 retrieve
```

memorph 是 **session conversion / migration / canonical archive layer**：

```text
provider A session
-> import 成 canonical session
-> preserve / active compression / archive
-> export 到 provider B native/portable session
-> target agent 继续使用
```

因此两者的数据生命周期不同：

| 维度 | Headroom | memorph |
|---|---|---|
| 所在层 | 请求发出前 | 会话文件/中间态转换 |
| 目标 | 节省当前请求 tokens | 跨 agent 会话可迁移、可压缩、可恢复 |
| 原文存储 | cache/store，默认 TTL 常较短 | durable archive，必须长期可恢复 |
| 恢复方式 | `headroom_retrieve(hash, query?)` | `memorph compression retrieve ARCHIVE_REF --query ...` |
| 压缩粒度 | live-zone block / tool output | canonical events / ranges / provider compressed blocks |
| 风险点 | provider cache 被破坏 | 目标 agent 把压缩块当普通上下文、或无条件展开 |

结论：memorph 可以学习 Headroom 的“压缩纪律”，但不能照搬它的 runtime cache 生命周期。

---

## 3. Headroom 会话相关设计的演进：旧模型 vs 新模型

### 3.1 旧模型：IntelligentContext / RollingWindow / message dropping

普通 docs 中仍能看到旧设计：

- `context-management.mdx` 描述 100-message conversation 超预算后，会给每条 message 打 importance score。
- 低分 message 被 drop。
- drop 的 message 放入 CCR。
- 插入 marker：“60 messages dropped, retrieve: def456”。

打分因子包括：

- recency。
- semantic similarity。
- TOIN importance。
- error indicator。
- forward reference。
- token density。

RollingWindow fallback 的规则是：

- 保留 system prompt。
- 保留最近 N 轮。
- tool call 和 tool response 原子删除，避免 orphan tool data。

这套设计从产品角度听起来合理，但 Headroom 自己后来在 REALIGNMENT 中否定了它。

### 3.2 新模型：live-zone-only compression

`REALIGNMENT/00-overview.md` 明确写出旧模型的错误：

```text
wrong mental model:
compression means choosing what to drop from conversation history

correct mental model:
passthrough is sacred;
compress only the live zone;
type-aware;
hash-keyed;
position-preserving;
with side-channel metadata;
cache hot zone never touched.
```

原因是对 provider cache 而言，历史消息、system prompt、tool definitions、thinking blocks 等已经进入 prompt cache hot zone。如果 Headroom 为了压缩重新序列化、删除、改写这些字节，会造成：

- Anthropic prompt cache 命中率下降。
- OpenAI prefix cache 被破坏。
- 请求 fingerprint 改变。
- subscription/OAuth traffic 可能暴露 wrapper 痕迹。
- reasoning/thinking/redacted/compaction 类型被破坏。

### 3.3 对 memorph 的判断

memorph 不完全受 provider cache hot zone 约束，因为 memorph 改的是 session 文件和 canonical 中间态。但这条原则仍然重要：

- 当前用户任务目标不能压没。
- 最近用户问题和最新 assistant 结论不能压没。
- provider 内部控制块不能错误改写。
- 已经 native compacted 的块不能展开成普通文本再传给目标 agent。
- portable compressed block 的 retrieval hint 是元信息，不能污染 summary。

memorph 当前已经在 Phase 25/26 中做了相关收口：

- provider projection 保留 archive ref 和 query-first hint。
- portable normalize 时跳过 retrieval hint，避免 hint 混进 summary。

---

## 4. CCR：Headroom 最值得学习的核心

### 4.1 CCR 的定义

CCR = Compress, Cache, Retrieve。

Headroom 的设计是：

```text
原始大内容
-> 压缩成短内容
-> 原始内容存入 compression store
-> prompt 中嵌入 hash marker
-> LLM 如果需要细节，调用 retrieve tool
-> runtime 从 store 查回原文或 query 命中子集
```

在 `docs/content/docs/ccr.mdx` 中，CCR 被描述为让 compression “reversible”。核心思想不是“压缩一定无损”，而是“线上 prompt 可以丢细节，但系统整体不丢原文”。

### 4.2 Rust CCR 的实现

`crates/headroom-core/src/ccr/mod.rs` 定义：

- `CcrStore` trait：
  - `put(hash, payload)`
  - `get(hash) -> Option<String>`
  - `len()`
- 默认容量：`DEFAULT_CAPACITY = 1000`
- 默认 TTL：`DEFAULT_TTL = 300s`
- hash：
  - BLAKE3。
  - 取前 24 个 hex 字符。
  - 96-bit hash space。
- marker：
  - `<<ccr:HASH>>`

Rust 注释明确说：这是 CCR 的 cornerstone，wire 上可以 lossy，但 end-to-end 必须 lossless。

### 4.3 CCR backends

Rust 文档描述三类 backend：

- `InMemoryCcrStore`
  - 进程本地。
  - 测试默认。
  - 重启丢失。
  - 多 worker 不共享。
- `SqliteCcrStore`
  - 生产默认。
  - 持久化。
  - 支持 worker 共享同一 DB 文件。
  - WAL。
  - lazy TTL purge。
- `RedisCcrStore`
  - feature gated。
  - 多 worker opt-in。
  - 不需要 sticky session。

### 4.4 Python CompressionStore 的实现

`headroom/cache/compression_store.py` 是 Python 层的 CCR store：

- `CompressionEntry` 保存：
  - `hash`
  - `original_content`
  - `compressed_content`
  - original/compressed token count
  - original/compressed item count
  - tool name / tool call id
  - query context
  - TTL
  - tool signature hash
  - compression strategy
  - retrieval count
  - search queries
  - last accessed
- 默认 TTL 也是 300 秒。
- 默认容量 1000。
- hash 默认 SHA-256(original) 前 24 hex。
- 支持 explicit hash，以兼容别的 producer 已经产生的 hash。
- 有 hash collision detection。
- 有 retrieval event。
- 有 redaction，避免 retrieval log 泄漏 secret/token/api key。
- 有 BM25 search。

### 4.5 Search retrieval

Python store 的 `search(hash, query, max_results, score_threshold)`：

- 先读取原始 entry。
- 将 original_content 规范化成 searchable items：
  - JSON array：每个 item。
  - JSON object：leaf records。
  - string/plain text：按行或 word window 切 chunk。
  - scalar：包装成 `{type, value}`。
- 使用 BM25 打分。
- 按 score 降序。
- 只返回 max_results。
- 记录 retrieval event。

### 4.6 CCR response handler

`tests/test_ccr_response_handler.py` 显示：

- 支持 Anthropic 格式：
  - `content` 数组中 `type: "tool_use"`，`name == headroom_retrieve`。
- 支持 OpenAI 格式：
  - `choices[].message.tool_calls[].function.name == headroom_retrieve`。
- 支持 full retrieval：
  - 只传 hash。
- 支持 search retrieval：
  - 传 hash + query。
- 支持 mixed tool calls：
  - CCR tool call 和其他 tool call 可以同时存在。

这说明 Headroom 把 retrieve 当成 runtime 内部工具，甚至可以透明处理继续调用。

### 4.7 Tool injection

`tests/test_ccr_tool_injection.py` 显示：

- `headroom_retrieve` tool schema 对不同 provider 有不同格式：
  - Anthropic：`name` + `input_schema`。
  - OpenAI：`type: function` + `function.parameters`。
  - Google：`parameters`。
- 扫描 marker：
  - 从 messages 中找 hash。
  - 支持 Anthropic content block。
  - 支持去重。
- 检测到 compressed marker 后注入 retrieval tool。
- 如果 tool 已存在则不重复注入。
- 可选注入 system instructions。

### 4.8 Headroom 设计中的问题

REALIGNMENT 指出旧 CCR tool injection 有 cache risk：

- 旧逻辑只在检测到压缩内容时注入 tool。
- tool array 会在不同请求之间变化。
- 这会破坏 provider 的 tool definition cache。

新建议是：

- `ccr_retrieve` 应该 always-on。
- 不应该因为本轮有没有压缩内容而改变 tool array。
- marker 和 store key 必须稳定。

### 4.9 memorph 应如何学习 CCR

memorph 已经不适合用 Headroom 的短 TTL cache，而应该使用 durable archive：

```text
Headroom:
hash -> short-lived cache/store -> headroom_retrieve(hash)

memorph:
memorph-archive://group/archive.json.gz
-> durable archive
-> memorph compression retrieve ARCHIVE_REF --query ...
```

memorph 应保留的原则：

- 原始 events 不丢。
- compressed summary 只作为 model-visible hint。
- archive ref 是 source of truth。
- 默认 query-first。
- full retrieval 必须显式。
- retrieval response 要说明 partial/full。
- retrieve result 要带 coverage metadata。

memorph 已经部分落地：

- `returned_event_ids`
- `omitted_event_count`
- `retrieval_mode`
- `recommended_next_action`
- query scoring guidance
- provider projection retrieval hint

---

## 5. Live-zone：Headroom 对“不要动历史”的工程化表达

### 5.1 live-zone 的定义

在 `crates/headroom-core/src/transforms/live_zone.rs` 中，live zone 是：

- 对 Anthropic `/v1/messages`：
  - `frozen_message_count` 以下是 prompt cache hot zone。
  - 最新 user message 是 live zone。
  - 最新 assistant message 是 cache hot zone，不动。
  - 最新 user message 内的 blocks 是候选。
  - 常见可压缩 block 是 `tool_result` 和长 `text`。

### 5.2 hot zone 的类型

`HOT_ZONE_BLOCK_TYPES` 包括：

- `tool_use`
- `thinking`
- `redacted_thinking`
- `compaction`

这些块即使在最新消息里，也不应该压。

REALIGNMENT 还扩展说明系统级 hot zone：

- system prompt。
- tools array。
- frozen historical messages。
- reasoning items with encrypted content。
- thinking blocks with signature。
- redacted_thinking data。
- compaction items。

### 5.3 对 memorph 的映射

memorph 不是每次请求的 live-zone，但有类似的“保护区”：

- 最新用户问题。
- 最新 assistant 结论。
- 当前任务目标。
- workspace 和 provider state。
- system/developer/AGENTS.md 指令。
- tool definitions/provider internal payload。
- native compressed marker。
- already compressed block 的 archive ref。

主动压缩时应该优先压：

- 旧的长 tool output。
- 旧的 search results。
- 旧的 build/test logs。
- 重复解释。
- 已经不影响当前任务的长历史。

不应该压：

- 最近 N 轮。
- 当前 goal。
- 未解决 TODO。
- 用户最新约束。
- provider native compaction marker 元信息。

---

## 6. Byte fidelity / cache safety：Headroom 很强调，但 memorph 要转译

### 6.1 Headroom 的 byte-range surgery

Headroom Rust live-zone dispatcher 不用“反序列化 JSON -> 改 Value -> 重新序列化整个 body”的方式，因为这会改变：

- whitespace。
- key order。
- numeric formatting。
- UTF-8 escaping。
- provider cache key。

它使用 `serde_json::value::RawValue` 保留原始字节片段，然后只对需要替换的 block 做 byte-range surgery：

```text
out = body[..block_start] + replacement + body[block_end..]
```

### 6.2 为什么重要

对 proxy 来说，任何未压缩部分都必须 byte-equal。否则 compression off/on 会破坏缓存、改变行为，甚至造成 provider-specific streaming/reasoning 字段丢失。

### 6.3 memorph 的等价要求

memorph 处理 session 文件，不一定要 byte-equal 原始 provider 文件；但是必须做到：

- canonical 表达稳定。
- provider payload 不随便丢。
- 未触碰的 events 不被无意义重写。
- provider-native compressed block 不被误展开/误普通化。
- portable compressed text normalize 不污染 summary。
- export 到目标 provider 时只按明确 projection 改写。

memorph 的“byte fidelity”应转译为：

```text
semantic fidelity + provider payload preservation + explicit projection boundary
```

---

## 7. ContentRouter / content detection：Headroom 的类型驱动压缩

### 7.1 Headroom 检测的内容类型

`crates/headroom-core/src/transforms/content_detector.rs` 定义：

- `JsonArray`
- `SourceCode`
- `SearchResults`
- `BuildOutput`
- `GitDiff`
- `Html`
- `PlainText`

检测顺序：

1. empty -> PlainText 0.0
2. JSON array
3. Git diff
4. HTML
5. Search results
6. Build/log output
7. Source code
8. PlainText fallback

检测方式：

- regex。
- structure。
- confidence threshold。
- 不依赖 LLM。
- 不做 I/O。

### 7.2 类型到 compressor 的映射

Rust live-zone dispatcher 使用：

- JSON array -> SmartCrusher。
- BuildOutput -> LogCompressor。
- SearchResults -> SearchCompressor。
- GitDiff -> DiffCompressor。
- SourceCode / PlainText / Html 暂时可能 no-op 或后续接 compressor。

### 7.3 LogCompressor 的机制

`log_compressor.rs` 说明：

1. 检测 format：
   - pytest
   - npm
   - cargo
   - jest
   - make
   - generic
2. 每行分类：
   - ERROR
   - FAIL
   - WARN
   - INFO
   - DEBUG
   - TRACE
3. 识别 stack trace。
4. 识别 summary line。
5. 每行打分。
6. adaptive total-lines budget。
7. 保留：
   - first/last/top errors。
   - failures。
   - warnings。
   - stack traces。
   - summaries。
   - context window。
8. 可选 CCR。

### 7.4 对 memorph 的启发

memorph 的主动压缩不应该只有一个“总结器”。应该至少建立：

```text
canonical event/block
-> visible text / provider payload extraction
-> type detection
-> candidate strategy
-> size/token gate
-> archive original events
-> emit compressed block
```

建议 memorph 的类型策略：

- Conversation summary：
  - 保留目标、约束、决策、未完成事项。
- Tool/log：
  - 保留命令、exit code、error、stack trace、最后 N 行、关键路径。
- Search result：
  - 保留 query、命中文件、关键行、数量统计。
- Diff/code：
  - 默认保守，只做摘要，不丢完整 archive。
- Provider native compaction：
  - preserve metadata，不展开成普通文本。

---

## 8. Token validation gate：Headroom 的“不能越压越大”标准

### 8.1 Headroom 的 gate

`live_zone.rs` 中每次 compressor 产出 candidate 后，会：

1. 计算 original tokens。
2. 计算 compressed tokens。
3. 如果 `compressed_tokens >= original_tokens`：
   - 不替换。
   - manifest 记 `RejectedNotSmaller`。
4. 只有 token 真变小才进入 `Compressed`。

同时还有 per-content-type byte threshold：

- JSON array。
- build/log。
- search results。
- git diff。
- source code。
- plain text。
- html。

低于阈值就 `BelowByteThreshold`，不启动 compressor。

### 8.2 manifest 中的 outcome

`BlockAction` 包括：

- `NoCompressionApplied`
- `Compressed`
- `CompressorError`
- `RejectedNotSmaller`
- `BelowByteThreshold`
- `Excluded`

这正是 memorph 用户要求的“有明确标准，不是没底线简化”。

### 8.3 memorph 对应要求

memorph 主动压缩必须有：

- dry-run report。
- candidate list。
- skip reasons。
- rejected reasons。
- size/token estimate。
- archive refs。
- source event ids。
- protected recent range。
- provider target。

已经应继续保持：

- 默认 opt-in。
- size gate。
- archive backed。
- summary 太长则拒绝。
- external summarizer 不可靠时 fallback deterministic summary。

---

## 9. CompressionManifest：Headroom 的可审计结果

### 9.1 Manifest 字段

`CompressionManifest` 包括：

- `messages_total`
- `messages_below_frozen_floor`
- `latest_user_message_index`
- `block_outcomes`

`BlockOutcome` 包括：

- message index。
- block index。
- block type。
- action。

manifest 还能计算：

- tokens_saved。
- transforms_applied。
- no-change reason。

### 9.2 memorph 的对应字段

memorph 需要比 Headroom 更会话化：

- `candidate_id`
- event range
- source event ids
- roles
- original bytes/tokens
- compressed bytes/tokens
- protected recent count
- strategy
- summary source
- archive ref
- target provider projection
- skip/reject reason
- risk level
- retrieval instructions

当前 memorph 已有一部分。后续还应保证：

- 每次 switch/apply 的 report 能持久查看。
- CLI/API/TUI 一致展示。
- 用户能知道“压了什么、没压什么、为什么”。

---

## 10. Memory：Headroom 的跨会话长期记忆

### 10.1 Memory 不是 session compression，但很相关

Headroom memory 文档把它称为 “temporal compression”：

```text
不是带着 10000 tokens 历史走，
而是提取 100 tokens 的记忆。
```

这和 memorph 的会话压缩不同：

- memory 是抽取事实/偏好/决策。
- compression archive 是保留原文并压缩可见上下文。

两者可以互补，但不能混为一个模块。

### 10.2 Memory scopes

`headroom/memory/models.py` 定义：

- USER：跨所有 sessions。
- SESSION：当前 task/conversation。
- AGENT：当前 agent lifetime。
- TURN：单次 LLM call。

`Memory` 字段包括：

- id
- content
- user_id
- session_id
- agent_id
- turn_id
- created_at
- valid_from
- valid_until
- importance
- supersedes / superseded_by
- promoted_from / promotion_chain
- access_count
- last_accessed
- entity_refs
- embedding
- metadata

### 10.3 Temporal versioning

Memory 支持 supersession chain：

- 旧事实不直接删除。
- 新事实 supersede 旧事实。
- query current state 默认排除 superseded。
- 可以查看 full chain。

这对 memorph 的启发：

- session compression archive 不应该覆盖旧 archive。
- 重新压缩同一段历史时，需要 lineage：
  - supersedes archive ref。
  - created_from source archive refs。
  - source_event_ids。
  - projection target。
- 这样才能审计“当前压缩块从哪来”。

### 10.4 Memory injection 的风险

`memory_handler.py` 里有两个模式：

- `AUTO_TAIL`
  - request 入口检索 memory。
  - append 到最新 user message tail。
  - 不改 system/hot zone。
- `TOOL`
  - 不自动注入。
  - model 显式调用 `memory_search`。

`memory_injection.py` 加了预算：

- 默认 1024 tokens。
- 默认最多 10 entries。
- min similarity 0.3。

这对 memorph 的启发：

- 如果后续把 compressed archive 的 retrieval 结果注入给目标 agent，也必须有预算。
- 不应该把 full archive 自动注入。
- query retrieval 结果要有 max results。
- full retrieval 应该只用于用户明确要求或 query 不够。

---

## 11. SharedContext：跨 agent handoff 的压缩上下文

### 11.1 SharedContext 做什么

`docs/content/docs/shared-context.mdx` 描述：

```text
Agent A stores large output
-> SharedContext.put(key, content, agent)
-> store original
-> compress content
Agent B get(key)
-> 默认拿 compressed
Agent B get(key, full=True)
-> 显式拿 original
```

这和 memorph 的跨 agent session switching 很接近。

### 11.2 SharedContext 的 API 形态

- `put(key, content, agent?)`
- `get(key, full?)`
- `stats()`
- `keys()`
- `clear()`

entry 记录：

- originalTokens。
- compressedTokens。
- savingsPercent。
- transforms。

### 11.3 对 memorph 的启发

memorph 的中间态可以看成更强版 SharedContext：

```text
canonical session
-> compressed blocks
-> durable archives
-> provider projections
```

可学习点：

- 默认返回 compressed。
- full 必须显式。
- entry 要有 stats。
- key/archive ref 要稳定。
- agent provenance 要记录。
- handoff 时不要无条件展开。

但 memorph 不应照搬内存 TTL store。session archive 必须 durable。

---

## 12. Provider 相关设计

### 12.1 Headroom 支持多 provider

README 和代码中涉及：

- Anthropic。
- OpenAI。
- Gemini。
- Bedrock。
- Vertex。
- Copilot CLI。
- Claude Code。
- Codex。
- Cursor。
- Aider。
- OpenClaw。

Provider-specific 行为在 `headroom/providers/`、`headroom/proxy/handlers/` 和 Rust proxy handlers 中。

### 12.2 Provider 差异影响 live-zone

`live_zone.rs` 注释说明不同 provider request shape 不同：

- Anthropic `/v1/messages`
  - messages。
  - user content blocks。
  - tool_result 嵌套在 user message。
- OpenAI Chat Completions
  - tool results 是独立 role=tool message。
- OpenAI Responses
  - input items。
  - function_call_output。
  - reasoning。
- Gemini
  - contents/parts/function_response。
- Bedrock
  - native embedded payload。

因此 Headroom 不做一个通用 walker，而是 provider-specific dispatcher。

### 12.3 memorph 的对应问题

memorph 也不能只做一个 generic compressed text：

- Codex 有 native `type: "compacted"`。
- OpenCode 有 native compaction part。
- Claude/Kimi/DeepSeek/Cursor/Kiro 等需要 portable projection。
- 不同 agent 的压缩 marker 不同。
- 从 A 到 B 时，需要把 A 的压缩语义映射到 B 可理解的 native/portable 压缩块，而不是无条件展开。

memorph 当前已经建立：

- native adapter registry。
- provider support profile。
- Codex native import/export compacted。
- OpenCode native projection。
- portable compressed text fallback。
- query-first retrieval hint。

后续要继续确保新增 provider 时只需要实现 adapter/projection，而不改核心 compression 模块。

---

## 13. Headroom 的测试纪律

### 13.1 CCR tests

`ccr_roundtrip.rs` 锁定：

- lossy path 必须写 store。
- hash 必须 resolve。
- same input same hash。
- distinct inputs distinct hash。
- passthrough 不写 store。
- lossless win 不写 store。
- marker 中 hash 必须和 store key 一致。

### 13.2 live-zone tests

Headroom 有：

- live_zone_dispatch。
- live_zone_thresholds。
- live_zone_token_validation。
- live_zone_ccr。

测试的意图：

- 只压 latest user/live-zone。
- respect frozen_message_count。
- below threshold 不压。
- compressed token 不小则 reject。
- CCR marker 有 store。
- no store 时不注入 marker。
- bytes outside compressed block 不动。

### 13.3 memorph 应保持的测试矩阵

memorph 对应至少需要：

- active compression dry-run。
- active compression apply archive backed。
- provider native import/export。
- portable projection contains archive_ref。
- portable normalize does not pollute summary。
- retrieve query partial。
- retrieve full。
- no-match。
- returned_event_ids/omitted_event_count。
- CLI retrieve。
- API retrieve。
- provider switch with compression opt-in。
- target provider projection preserves compressed semantics。

当前 memorph 已经覆盖了其中很多项。后续新增 provider 或 compression strategy 时必须补同级测试。

---

## 14. 对 memorph 的具体改造路线建议

### 14.1 已经完成或正在形成的能力

根据当前 memorph 代码和路线文件，已经有：

- canonical `EventBlock::Compressed`。
- durable archive。
- `memorph-archive://...`。
- active compression plan/apply。
- archive restore/expand。
- provider-native projection。
- Codex native compacted import/export。
- OpenCode native compaction projection。
- portable compressed text projection。
- query-first CLI/API retrieval。
- retrieval tool spec/instructions。
- retrieval mode。
- returned event coverage。
- query scoring。
- provider projection retrieval hints。
- portable hint normalize bugfix。

这些方向与 Headroom 的 CCR/live-zone 纪律是对齐的。

### 14.2 还应继续补强的能力

#### 14.2.1 Parser 边界

portable compressed text 的 parser 应继续补：

- 多行 summary。
- summary 前后空行 trim。
- metadata line 在 summary 之后。
- retrieval hint 在 summary 之后。
- 空 summary 拒绝。
- malformed archive line 不污染 summary。

这是当前 Phase 27 的合理下一步。

#### 14.2.2 Archive lineage

如果同一段内容被二次压缩、跨 provider 往返，archive 应记录：

- created_from_archive_refs。
- supersedes_archive_ref。
- source_provider_id。
- target_provider_id。
- source_event_ids。
- projection type。
- compression strategy。

#### 14.2.3 Strategy router

主动压缩不应只做 generic summary：

- conversation summary strategy。
- log strategy。
- search result strategy。
- diff/code strategy。
- provider-native-preserve strategy。

#### 14.2.4 Retrieval budget

query retrieval 结果应继续避免过大：

- `max_results`。
- max bytes/tokens。
- snippet cap。
- full retrieval warning。
- omitted count。

#### 14.2.5 Manifest 和 user decision

TUI/API/CLI 应让用户看到：

- 将压哪些候选。
- 为什么压。
- 为什么跳过。
- 压缩后预计节省多少。
- archive 存在哪里。
- 是否可恢复。

### 14.3 不建议照搬的部分

#### 不建议照搬 Headroom TTL cache

Headroom 默认 5 分钟 TTL 适合 runtime request，但 memorph 是 session archive。memorph 需要 durable archive，不应过期。

#### 不建议照搬 old IntelligentContext

会话打分后删旧 messages 的模型已经被 Headroom 自己否定。memorph 可以做主动压缩 range selection，但不能变成不可审计的“把低分事件丢掉”。

#### 不建议默认自动 full expand

Headroom 的 retrieve 是按需。memorph 更应该 query-first。尤其跨 agent 切换时，目标 agent 不应无条件读完整旧历史。

#### 不建议把 memory 和 compression archive 混成一个库

memory 是长期事实抽取，archive 是原文恢复。两者都重要，但边界要清楚：

- memory 可抽象、可合并、可 supersede。
- archive 必须保真、可恢复、可审计。

---

## 15. 与 memorph 现有压缩机制的逐项映射

| Headroom 概念 | Headroom 做法 | memorph 对应 | 当前建议 |
|---|---|---|---|
| CCR store | hash -> original payload | archive_ref -> archived SessionEvents | 已对齐，继续 durable archive |
| marker | `<<ccr:HASH>>` | `memorph-archive://...` + retrieval hint | 已对齐 |
| retrieve tool | `headroom_retrieve(hash, query)` | CLI/API retrieve archive query | 不做 MCP，但 tool spec/instructions 可保留 |
| live zone | latest user/tool block | 压旧历史，保护最近 N 轮 | 已有 policy，继续强化 |
| hot zone | system/tools/history/thinking 不动 | provider internal/system/current task 不压 | 继续用 tests 锁定 |
| content detector | JSON/log/search/diff/code/text | active compression router | 后续重点 |
| token gate | compressed tokens < original | size/token estimate gate | 已有基础，后续 provider tokenizer |
| manifest | per block outcome | dry-run/apply report | 持续增强 |
| search retrieval | BM25 | lightweight scoring | 已有 phrase/coverage scoring，可后续增强 |
| shared context | compressed default/full explicit | compressed block default/retrieve explicit | 已对齐 |
| memory scopes | user/session/agent/turn | workspace/session/provider/agent | workspace 已一等能力，继续增强 |

---

## 16. 关键风险清单

### 16.1 压缩摘要污染

风险：retrieval hint、archive metadata、provider marker 混进 summary。

当前 memorph 已修复 portable hint normalize 的一个污染点。后续 parser 边界还要继续补。

### 16.2 无条件展开

风险：从 A 切到 B 时，把 compressed archive 全部展开，导致上下文更长甚至 429。

应对：

- provider projection 保持 compressed block。
- query-first hint。
- retrieval mode。
- omitted count。
- full retrieval 警告。

### 16.3 archive 丢失或不可恢复

风险：压缩块只有 summary，没有原文。

应对：

- apply 前 archive 写成功。
- archive ref 写入 compressed block。
- archive loader tests。
- retrieve CLI/API tests。

### 16.4 target provider 不理解 compressed block

风险：目标 agent 把 portable marker 当普通上下文。

应对：

- native projection 优先。
- portable text 明确 marker + archive + query-first command。
- provider tests 覆盖所有 projection。

### 16.5 query retrieval 命中质量差

风险：用户问“最初目标是什么”，query 命不中关键历史。

应对：

- query scoring。
- snippet。
- returned_event_ids。
- omitted_event_count。
- future：BM25 或 field weighting。

### 16.6 压缩过度

风险：为了变短丢掉当前任务目标、约束、TODO。

应对：

- protect recent N。
- current task markers。
- goal preservation。
- user-select candidates。
- dry-run report。

---

## 17. 推荐的 memorph 后续阶段

### Phase A：parser/portable 边界收口

目标：

- 多行 summary。
- metadata 行跳过。
- retrieval hint 行跳过。
- 空 summary 拒绝。
- malformed metadata 不污染。

验收：

- `parse_portable_compressed_text` 直接单测。
- provider normalize 测试。

### Phase B：active compression router 分层

目标：

- conversation/log/search/diff/code 的 candidate classification。
- report 中体现 strategy。

验收：

- 每种类型一个 fixture。
- skip/reject reason 可见。

### Phase C：archive lineage

目标：

- archive metadata 增加 lineage。
- 二次压缩可审计。

验收：

- archive JSON schema 测试。
- restore/expand 不变。

### Phase D：retrieval quality

目标：

- query scoring 更稳。
- 可选 BM25。
- snippet 更适合 event-level。

验收：

- query 排序测试。
- no-match/broaden query guidance。

### Phase E：user-select compression

目标：

- 用户可选候选。
- TUI/CLI/API 对齐。

验收：

- selected candidates only。
- skipped candidates preserved。

### Phase F：provider expansion

目标：

- 新增 provider 时只实现 adapter。
- 不改 core compression。

验收：

- provider support matrix。
- native/portable projection tests。

---

## 18. 最终判断

Headroom 对 memorph 的最大价值不是“把 Python 代码翻译成 Rust”，也不是“照搬它的 MCP/Memory/Proxy”，而是下面这套原则：

```text
1. 压缩必须可逆。
2. 原文必须有稳定引用。
3. 目标 agent 默认看压缩块，不默认看完整旧历史。
4. 需要细节时 query-first retrieve。
5. 当前任务状态和 provider hot zone 不动。
6. 每一次压缩都有 manifest。
7. 压缩后不小就拒绝。
8. provider projection 要显式表达 compressed semantics。
```

memorph 当前已经沿这条路线推进，并且比 Headroom 更适合做“跨 agent 会话压缩管理”，因为 memorph 有 canonical session、provider import/export、workspace、archive 和 session switching 的天然位置。

下一步不应该追求大而全模型压缩，而应该继续做：

- parser 边界。
- projection 不丢语义。
- archive lineage。
- type-aware active compression。
- retrieve quality。
- 用户可审计和可选择。

这才是把 Headroom 的有效理念学到 memorph，而不是做一个半拉子的压缩功能。
