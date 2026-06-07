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

---

## 19. 逐文件级源码索引：会话、压缩、恢复、provider 映射相关部分

这一章是前面结论的证据索引。这里的“逐文件级”不是把 Headroom 的每个无关 UI、安装包装、图片、lockfile 都机械展开，而是覆盖与会话压缩、CCR、live-zone、proxy 请求改写、memory 注入、provider adapter、relevance 检索、缓存稳定、可观测性和相关测试直接相关的文件。MCP 文件只作为设计资料说明，不作为 memorph 当前实现目标。

### 19.1 读源码的优先级

如果只读最核心文件，顺序应该是：

1. `crates/headroom-core/src/transforms/live_zone.rs`：新压缩核心，不压历史整段，只压 live-zone 候选。
2. `crates/headroom-core/src/ccr/mod.rs`：压缩内容引用和恢复的核心抽象。
3. `crates/headroom-core/src/transforms/content_detector.rs`：内容类型识别，决定不能用一个摘要策略处理所有内容。
4. `crates/headroom-core/src/transforms/pipeline/orchestrator.rs`：压缩流水线编排。
5. `crates/headroom-proxy/src/compression/live_zone_openai.rs`、`live_zone_anthropic.rs`、`live_zone_responses.rs`：把 core 压缩结果投影到不同 provider。
6. `headroom/cache/compression_store.py`、`headroom/ccr/context_tracker.py`：旧 Python 体系中“原文存储/引用追踪”的参考。
7. `headroom/memory/bridge_parsers.py`、`headroom/memory/sync_adapters/codex_agent.py`、`headroom/memory/sync_adapters/claude_code.py`：不同 agent 原生数据理解的参考。

对 memorph 的判断：Headroom 是请求前压缩，memorph 应该是“会话中间态压缩 + 跨 agent projection”。两者学习的是机制，不是部署形态。

### 19.2 `REALIGNMENT` 架构决策文件

| 文件 | 内容定位 | 会话/压缩相关性 | memorph 应学什么 |
| --- | --- | --- | --- |
| `REALIGNMENT/INDEX.md` | realignment 文档目录。 | 说明 Headroom 已经把重构拆成阶段，不是在旧 Python proxy 上继续堆功能。 | memorph 的压缩模块也要阶段化：先 archive/ref/projection，再主动压缩，再 LLM。 |
| `REALIGNMENT/00-overview.md` | 总体重构目标。 | 明确 Python proxy 路径混乱、压缩边界不稳、provider 兼容问题。 | 压缩必须成为独立模块，不能藏在某个 importer/exporter。 |
| `REALIGNMENT/01-bug-list.md` | 已知 bug 清单。 | 记录 cache、compression、proxy 行为不一致。 | 跨 agent 压缩失败往往是边界状态、标记、缓存和 provider 映射失败，不只是算法失败。 |
| `REALIGNMENT/02-architecture.md` | 新架构蓝图。 | 把 core、proxy、provider、observability 拆开。 | memorph 应保持 `canonical compression core -> provider projection -> runtime adapter`。 |
| `REALIGNMENT/03-phase-A-lockdown.md` | 冻结和测试基线。 | 动压缩前先锁定行为。 | memorph 需要 fixture：Claude 压缩后迁移 Codex、Kimi 压缩后迁移 DeepSeek、Codex 中再次 compact。 |
| `REALIGNMENT/04-phase-B-live-zone.md` | live-zone 压缩核心。 | 不压历史整段、不破坏 cache hot zone，只处理活跃区块。 | 这是 memorph 主动压缩的核心参考：压缩候选区和热区保护要分开。 |
| `REALIGNMENT/05-phase-C-rust-proxy.md` | Rust proxy 接管请求路径。 | 压缩从 Python 运行时转到更稳定的请求代理层。 | memorph 不一定做 HTTP proxy，但需要稳定边界：导入、导出、压缩、恢复不能散落在 CLI glue。 |
| `REALIGNMENT/06-phase-D-bedrock-vertex.md` | Bedrock/Vertex provider 扩展。 | 同一压缩 core 必须能投影到不同 provider envelope。 | Kimi、DeepSeek、Codex、Claude 的差异应在 adapter，不在 core compression。 |
| `REALIGNMENT/07-phase-E-cache-stabilization.md` | cache 稳定化。 | 关注 prompt cache 不被压缩扰动。 | 当前工作区状态、当前 agent 必要元信息、热上下文要定义为不可压缩区。 |
| `REALIGNMENT/08-phase-F-auth-mode.md` | 认证模式统一。 | 与压缩间接相关，影响 proxy 透明接入 provider。 | agent runtime concern 不应进入 compression core。 |
| `REALIGNMENT/09-phase-G-rtk-observability.md` | 可观测性。 | 压缩率、cache hit、漂移、失败原因都要可观测。 | compression manifest 要记录 skipped/rejected/reason/ratio/retrieval id。 |
| `REALIGNMENT/10-phase-H-python-retirement.md` | 旧 Python 退役计划。 | 旧 Python 模块不是最终形态。 | 不要照搬已被 Headroom 准备废弃的 Python `IntelligentContext/RollingWindow`。 |
| `REALIGNMENT/11-phase-I-test-infra.md` | 测试设施。 | provider 请求、压缩、cache、streaming 做成可测试单元。 | memorph 需要专门 compression switch fixtures。 |
| `REALIGNMENT/12-decisions-needed.md` | 未决决策。 | compression policy、cache 策略、provider 行为仍需取舍。 | 主动压缩是否启用、阈值、是否允许 LLM、是否保留原文要显式策略化。 |

### 19.3 文档站文件：可靠输入和需要谨慎的旧描述

| 文件 | 内容定位 | 会话/压缩相关性 | memorph 结论 |
| --- | --- | --- | --- |
| `docs/content/docs/ccr.mdx` | CCR 说明。 | 直接说明压缩内容引用、存储、恢复、tool 注入，并指出旧 message-level CCR 已 retired。 | 这是 `CompressedBlockRef + archive + retrieve` 的核心参考。 |
| `docs/content/docs/how-compression-works.mdx` | 压缩工作流。 | 描述 Headroom 如何判断内容类型、压缩、校验 token 节省。 | 可参考“先估算、再压缩、再验证”。 |
| `docs/content/docs/context-management.mdx` | 上下文管理。 | 包含旧 rolling context 思路，与 live-zone 新方向冲突。 | 只能作为反例，memorph 不应简单 drop 旧消息。 |
| `docs/content/docs/architecture.mdx` | 组件架构。 | 展示 proxy、compression、memory、cache 的组合方式。 | 可参考分层，但 memorph 以 agent migration 为主，不以 HTTP proxy 为主。 |
| `docs/content/docs/cache-optimization.mdx` | cache 优化。 | 解释缓存稳定、prefix、动态内容检测。 | 对 hot zone、workspace 一等能力有参考。 |
| `docs/content/docs/code-compression.mdx` | 代码压缩。 | 代码类型内容的保真压缩规则。 | 用于工具输出、patch、diff 压缩候选，不用于用户意图摘要。 |
| `docs/content/docs/text-and-logs.mdx` | 文本和日志压缩。 | 针对 log/search 输出的结构化压缩。 | 很适合 memorph 主动压缩旧工具输出。 |
| `docs/content/docs/smart-crusher.mdx` | Smart Crusher。 | 介绍非 LLM 的结构化压缩器。 | “无需调 LLM 的压缩”关键参考，适合第一阶段实现。 |
| `docs/content/docs/memory.mdx` | memory 功能。 | 长期记忆和上下文注入。 | 可参考检索，但不能混淆长期记忆和可逆会话压缩。 |
| `docs/content/docs/shared-context.mdx` | 跨工具共享上下文。 | 多工具/agent 共享记忆。 | 对 workspace 一等能力有参考，但压缩块不能直接变成泛化 memory。 |
| `docs/content/docs/proxy.mdx` | proxy 接入。 | 请求路径和 provider 代理。 | 即使 memorph 不做 HTTP proxy，也可学“请求前投影”的位置。 |
| `docs/content/docs/openai-sdk.mdx` | OpenAI SDK 接入。 | provider envelope 示例。 | 用于理解 Codex/OpenAI 类投影差异。 |
| `docs/content/docs/anthropic-sdk.mdx` | Anthropic SDK 接入。 | Anthropic cache control 和 message 格式。 | Claude 压缩迁移要保留原生 compact/cache 语义。 |
| `docs/content/docs/litellm.mdx` | LiteLLM 接入。 | 多 provider 统一入口。 | 统一接口不等于统一压缩语义。 |
| `docs/content/docs/mcp.mdx` | MCP 功能。 | Headroom 用 MCP 暴露 retrieve 等工具。 | 当前不实现 MCP，只借鉴 retrieve 协议形状。 |
| `docs/content/docs/api-reference.mdx` | API 参考。 | 压缩、memory、proxy API 入口。 | 可作为 memorph CLI/API 设计对照。 |
| `docs/content/docs/configuration.mdx` | 配置。 | 压缩阈值、provider、cache 等配置入口。 | compression policy 要显式配置化。 |
| `docs/content/docs/metrics.mdx` | 指标。 | 压缩率、节省、cache 命中。 | manifest 应能生成同类指标。 |
| `docs/content/docs/benchmarks.mdx` | benchmark。 | 评估压缩节省。 | memorph 应建立真实会话 fixture，而非只测合成文本。 |
| `docs/content/docs/failure-learning.mdx` | 从失败中学习。 | 与 memory/learning 相关。 | 可参考失败事件结构化，不把压缩恢复当 memory 学习。 |
| `docs/content/docs/filesystem-contract.mdx` | 文件系统约定。 | 存储路径和兼容性。 | archive/store 需要文件系统契约，特别是跨 workspace。 |
| `docs/content/docs/limitations.mdx` | 限制。 | 说明不能压缩/不应压缩的边界。 | memorph 也要明确“不压当前任务状态、不丢原文、不无条件展开”。 |
| `docs/content/docs/simulation.mdx` | 模拟/评估。 | 可用于压缩效果模拟。 | 可学习离线评估方法。 |
| `docs/content/docs/errors.mdx` | 错误处理。 | 压缩失败、provider 错误、配置错误。 | 区分“压缩失败但可继续”和“会话不可恢复”。 |
| `docs/content/docs/troubleshooting.mdx` | 排障。 | 用户可见行为说明。 | 最终要解释 compressed/retrieved/skipped。 |
| `docs/content/docs/image-compression.mdx` | 图像压缩。 | 与文本会话压缩无直接关系。 | 当前不纳入核心。 |
| `docs/content/docs/quickstart.mdx`、`installation.mdx`、`docker-install.mdx`、`persistent-installs.mdx` | 安装和入门。 | 与压缩语义弱。 | 不作为实现参考。 |
| `docs/content/docs/agno.mdx`、`langchain.mdx`、`strands.mdx`、`vercel-ai-sdk.mdx` | 外部框架接入。 | 展示不同 envelope。 | 只作为 provider/framework 差异资料。 |
| `docs/content/docs/community-savings.mdx`、`releases.mdx`、`index.mdx` | 统计、发布、首页。 | 与压缩核心弱。 | 辅助判断版本演进。 |

### 19.4 Rust core：压缩、CCR、token、relevance 的真实核心

#### 19.4.1 core 根模块和策略

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `crates/headroom-core/src/lib.rs` | Rust core 对外导出。 | 把 auth、cache_control、ccr、compression_policy、relevance、tokenizer、transforms 组合成稳定 API。 | 外部只应调用 `analyze -> compress -> project -> retrieve`，不要直接碰内部 detector。 |
| `crates/headroom-core/src/compression_policy.rs` | 压缩策略定义。 | 控制什么时候压、压到什么程度、如何选择 transform。 | 主动压缩阈值、最小收益、热区、LLM 开关都放 policy。 |
| `crates/headroom-core/src/cache_control.rs` | cache control 语义。 | 描述 provider cache 控制字段和稳定前缀。 | provider compact/cache 标记不能当普通文本。 |
| `crates/headroom-core/src/auth_mode.rs` | auth mode。 | provider 接入 concern。 | 不学 auth 实现，只学运行时 concern 与 compression core 分离。 |

#### 19.4.2 CCR core 和后端

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `crates/headroom-core/src/ccr/mod.rs` | CCR 类型和接口。 | 定义压缩引用、存储、恢复、backend trait。 | 压缩块必须是引用，不是不可追溯摘要。 |
| `crates/headroom-core/src/ccr/backends/mod.rs` | CCR backend 聚合。 | 后端细节隐藏在统一接口后。 | archive 后端可替换，不影响 provider projection。 |
| `crates/headroom-core/src/ccr/backends/in_memory.rs` | 内存 backend。 | 适合测试和短生命周期。 | 只适合单元测试，不适合作为真实 archive。 |
| `crates/headroom-core/src/ccr/backends/sqlite.rs` | SQLite backend。 | 本地持久化。 | memorph 长期压缩 archive 更接近这个方向。 |
| `crates/headroom-core/src/ccr/backends/redis.rs` | Redis backend。 | 共享/服务化场景。 | 当前不必实现，但说明 store 不应绑死文件结构。 |

#### 19.4.3 tokenizer

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `crates/headroom-core/src/tokenizer/mod.rs` | tokenizer 抽象。 | 统一 token 估算接口。 | 压缩是否成功必须以目标 provider/model 估算为准。 |
| `crates/headroom-core/src/tokenizer/estimator.rs` | fallback token 估算。 | 没有精确 tokenizer 时近似估算。 | manifest 要记录 estimator type，避免误判。 |
| `crates/headroom-core/src/tokenizer/tiktoken_impl.rs` | tiktoken 实现。 | OpenAI/Codex 类模型估算。 | Codex projection 优先使用接近目标模型的 tokenizer。 |
| `crates/headroom-core/src/tokenizer/hf_impl.rs` | HuggingFace tokenizer。 | 非 OpenAI 模型。 | Kimi/DeepSeek 可能需要独立 tokenizer 配置。 |
| `crates/headroom-core/src/tokenizer/registry.rs` | tokenizer 注册表。 | 根据模型/provider 选 tokenizer。 | 需要 provider/model -> tokenizer registry。 |

#### 19.4.4 transform 入口、检测和安全

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `crates/headroom-core/src/transforms/mod.rs` | transform 聚合。 | 统一导出 live_zone、detector、compressor、pipeline。 | compression transform 应独立成模块。 |
| `crates/headroom-core/src/transforms/content_detector.rs` | 内容类型检测。 | 识别 log、search、diff、code、json。 | 主动压缩第一阶段应类型感知，而不是通用摘要。 |
| `crates/headroom-core/src/transforms/detection.rs` | detection 通用类型。 | 提供检测结果结构。 | `content_type/confidence/evidence` 要写入 manifest。 |
| `crates/headroom-core/src/transforms/magika_detector.rs` | Magika 检测集成。 | 外部 detector 提升类型识别。 | 当前不必依赖 Magika，但保留 detector adapter 思路。 |
| `crates/headroom-core/src/transforms/unidiff_detector.rs` | unified diff 检测。 | 精确识别 diff。 | 对 patch/代码审查输出很关键。 |
| `crates/headroom-core/src/transforms/tag_protector.rs` | 标签保护。 | 保护 XML/marker/tag 不被误压缩。 | 必须保护 provider 原生 compact/summary marker、tool call id、archive ref。 |
| `crates/headroom-core/src/transforms/safety.rs` | 安全门。 | 防止危险或无效压缩。 | 不安全就 reject，不强压。 |
| `crates/headroom-core/src/transforms/recommendations.rs` | 推荐/配置加载。 | 根据内容或模型给压缩建议。 | 可作为 compression candidate recommendation 参考。 |
| `crates/headroom-core/src/transforms/adaptive_sizer.rs` | 自适应压缩大小。 | 根据预算选择压缩目标。 | 按目标 agent 上下文预算决定压缩强度。 |
| `crates/headroom-core/src/transforms/anchor_selector.rs` | anchor 选择。 | 保留关键锚点。 | 任务目标、决策、文件路径、错误、命令、结果都应是 anchor。 |

#### 19.4.5 live-zone 和专用压缩器

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `crates/headroom-core/src/transforms/live_zone.rs` | 新核心压缩策略。 | 不移动历史消息，不压 cache hot zone，只压 live-zone 中大型可压缩 block；生成 outcome/manifest；做 token validation。 | 跨 agent 切换时，应让目标 agent 看到压缩引用和必要摘要，不展开原文；只有 query 需要时 retrieve。 |
| `crates/headroom-core/src/transforms/log_compressor.rs` | 日志压缩。 | 保留错误、异常、代表性行，去掉重复噪声。 | 主动压缩最适合从日志/命令输出开始。 |
| `crates/headroom-core/src/transforms/search_compressor.rs` | 搜索结果压缩。 | 保留 query、命中路径、关键片段，减少重复结果。 | `rg/find/search` 结果可物理压小，同时保留 archive。 |
| `crates/headroom-core/src/transforms/diff_compressor.rs` | diff 压缩。 | 保留文件、hunk、关键变更，去掉低价值上下文。 | patch 压缩不能丢改动语义。 |

live-zone 到 memorph 的具体映射：

| Headroom 概念 | memorph 映射 |
| --- | --- |
| hot zone | 当前任务目标、最新用户问题、最新 agent 计划、workspace state、最近工具结果、provider 必须标记 |
| live-zone candidate | 旧日志、旧搜索结果、大段 diff、重复工具输出、旧压缩块的二次投影 |
| CCR ref | `CompressedBlockRef` 或 archive id |
| token validation | import/export 前后 token 估算 |
| reject compression | 保留原文并记录 `skip_reason` |
| content router | 按 event/tool/message 类型选择 compressor |

#### 19.4.6 pipeline

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `transforms/pipeline/mod.rs` | pipeline 聚合。 | 组织 offload、reformat、orchestrator。 | 需要统一 `CompressionPipeline`。 |
| `transforms/pipeline/config.rs` | pipeline 配置。 | 控制启用哪些阶段。 | 可配置“仅非 LLM 压缩/允许 LLM/仅用户选中候选”。 |
| `transforms/pipeline/traits.rs` | pipeline trait。 | 定义 stage 输入输出。 | 分离 detector、compressor、validator、archiver。 |
| `transforms/pipeline/orchestrator.rs` | 编排器。 | 决定走哪个 stage，收集结果。 | 核心落点：统一生成 manifest，不让 adapter 自己决定成败。 |
| `transforms/pipeline/offloads/mod.rs` | offload 聚合。 | 将大块内容替换为引用或压缩 representation。 | “物理层面压缩”就是 canonical event 中替换为 archive ref。 |
| `transforms/pipeline/offloads/diff_offload.rs` | diff offload。 | 大 diff 转外部引用。 | 大 diff 原文进 archive，中间态只留 summary/ref。 |
| `transforms/pipeline/offloads/diff_noise.rs` | diff 噪声识别。 | 识别低价值 diff 上下文。 | 避免压掉真正有意义 hunk。 |
| `transforms/pipeline/offloads/json_offload.rs` | JSON offload。 | 大 JSON 转引用或精简。 | 工具调用返回大 JSON 时很适合。 |
| `transforms/pipeline/offloads/log_offload.rs` | log offload。 | 大日志转引用。 | 命令输出压缩可直接借鉴。 |
| `transforms/pipeline/offloads/search_offload.rs` | search offload。 | 搜索结果转引用/摘要。 | 优先覆盖。 |
| `transforms/pipeline/reformats/mod.rs` | reformat 聚合。 | 可逆或低风险格式压缩。 | 先 deterministic reformat，再考虑 LLM。 |
| `transforms/pipeline/reformats/json_minifier.rs` | JSON minify。 | 删除空白但保留结构。 | 低风险无损物理压缩入口。 |
| `transforms/pipeline/reformats/log_template.rs` | log 模板化。 | 将重复 log 行抽成模板。 | 对长日志收益大，比摘要更可审计。 |

#### 19.4.7 Smart Crusher

Smart Crusher 是 Headroom 中非 LLM 压缩理念最集中的部分。它不是“模型总结”，而是统计、分类、锚点、结构化压缩。

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `transforms/smart_crusher/mod.rs` | 模块聚合。 | 对外暴露 crusher。 | 可做 `smart_compressors` 子模块。 |
| `transforms/smart_crusher/config.rs` | 配置。 | 控制阈值、预算、策略。 | 压缩强度必须配置化。 |
| `transforms/smart_crusher/types.rs` | 类型定义。 | 定义 block、segment、result。 | 中间态需要压缩候选结构化类型。 |
| `transforms/smart_crusher/traits.rs` | trait。 | 抽象 analyzer/crusher。 | 新增 agent 投影不改 core。 |
| `transforms/smart_crusher/analyzer.rs` | 内容分析。 | 统计行、重复、重要性。 | 压缩前先分析，不直接摘要。 |
| `transforms/smart_crusher/classifier.rs` | 内容分类。 | 判断内容形态。 | 与 content detector 配合。 |
| `transforms/smart_crusher/crusher.rs` | 主压缩器。 | 执行压缩决策。 | 主动压缩执行器参考。 |
| `transforms/smart_crusher/crushers.rs` | 多 crusher 集合。 | 不同内容类型对应不同 crusher。 | 不要一个 summary prompt 处理所有内容。 |
| `transforms/smart_crusher/builder.rs` | builder。 | 构造 crusher 配置。 | 简化 policy 注入。 |
| `transforms/smart_crusher/orchestration.rs` | 编排。 | 组合分析、计划、压缩。 | 和 pipeline orchestrator 共同形成完整流程。 |
| `transforms/smart_crusher/planning.rs` | 压缩计划。 | 先计划再执行。 | 用户可选压缩候选时，需要展示 plan。 |
| `transforms/smart_crusher/anchors.rs` | 锚点。 | 保留关键行/片段。 | 任务目标、错误、文件路径、命令、结论都应作为 anchor。 |
| `transforms/smart_crusher/constraints.rs` | 约束。 | 防止违反保真边界。 | “不丢有意义上下文”要落成约束。 |
| `transforms/smart_crusher/error_keywords.rs` | 错误关键词。 | 错误行提权。 | 命令输出压缩不能丢 error/warn/stacktrace。 |
| `transforms/smart_crusher/field_detect.rs` | 字段检测。 | 识别结构化字段。 | 对 JSON/tool result 保留关键字段。 |
| `transforms/smart_crusher/hashing.rs` | 哈希。 | 内容去重、引用稳定。 | archive id 应有稳定指纹。 |
| `transforms/smart_crusher/observer.rs` | 观察/事件。 | 暴露压缩过程。 | 需要 compression trace。 |
| `transforms/smart_crusher/outliers.rs` | 异常值。 | 找出少数重要行。 | 长日志里异常行通常比平均行重要。 |
| `transforms/smart_crusher/statistics.rs` | 统计。 | 行频、长度、重复模式。 | 决定压缩候选的基础。 |
| `transforms/smart_crusher/stats_math.rs` | 统计计算。 | 底层数学工具。 | 可借鉴，不必照搬。 |
| `transforms/smart_crusher/compaction/mod.rs` | compaction 聚合。 | Smart Crusher 内部二级压缩。 | 后续可做二次压缩 lineage。 |
| `transforms/smart_crusher/compaction/ir.rs` | 中间表示。 | 原文转成可压缩 IR。 | canonical event 本身应承担 IR 角色。 |
| `transforms/smart_crusher/compaction/classifier.rs` | compaction 分类。 | 判断 IR 如何压缩。 | 与 event type mapping 一致。 |
| `transforms/smart_crusher/compaction/compactor.rs` | compaction 执行。 | 执行压缩。 | 执行阶段参考。 |
| `transforms/smart_crusher/compaction/formatter.rs` | 输出格式化。 | 生成模型可读压缩文本。 | provider projection 需要类似 formatter。 |
| `transforms/smart_crusher/compaction/walker.rs` | IR walker。 | 遍历结构。 | 对嵌套 tool result/JSON 有用。 |

#### 19.4.8 signals 和 relevance

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `signals/README.md` | signals 说明。 | 解释重要性信号。 | 压缩前先评估重要性。 |
| `signals/mod.rs` | signals 聚合。 | 暴露 keyword、line importance、tiered。 | 重要性评估独立成模块。 |
| `signals/keyword_detector.rs` | 关键词检测。 | 识别 error、todo、file path。 | 低成本重要性判断。 |
| `signals/line_importance.rs` | 行重要度。 | 给长文本每行打分。 | 日志/搜索结果压缩必需。 |
| `signals/tiered.rs` | 分层信号。 | 将内容分层保留。 | 可定义 must_keep/summary_only/archive_only。 |
| `relevance/mod.rs` | relevance 聚合。 | 对外导出检索能力。 | archive retrieve 需要相关性排序。 |
| `relevance/base.rs` | 基础接口。 | 定义 scorer/searcher。 | 不应直接把 archive 全展开。 |
| `relevance/bm25.rs` | BM25。 | 无 embedding 的关键词检索。 | 第一阶段最适合，低依赖、可解释。 |
| `relevance/embedding.rs` | embedding 检索。 | 语义检索。 | 后续可选，不应第一阶段硬依赖。 |
| `relevance/hybrid.rs` | 混合检索。 | BM25 和 embedding 结合。 | 适合高质量恢复。 |

### 19.5 Rust proxy：真实请求如何被压缩和投影

| 文件 | 角色 | 会话/压缩相关性 | memorph 学习点 |
| --- | --- | --- | --- |
| `crates/headroom-proxy/src/lib.rs` | proxy crate 导出。 | 组合 config、handlers、compression、observability。 | adapter 层只组合模块，不拥有压缩逻辑。 |
| `crates/headroom-proxy/src/main.rs` | proxy 入口。 | 启动服务。 | 学启动时注入 policy/config，不学服务形态。 |
| `crates/headroom-proxy/src/config.rs` | proxy 配置。 | provider、endpoint、compression 开关。 | agent profile 应包含 compression capability。 |
| `crates/headroom-proxy/src/error.rs` | 错误类型。 | 区分 provider/proxy/compression 错误。 | 区分 import/export/compress/retrieve/project 错误。 |
| `crates/headroom-proxy/src/proxy.rs` | 请求转发核心。 | 在请求进入 provider 前改写/压缩/记录。 | memorph 对应位置是“导出到目标 agent 前”。 |
| `crates/headroom-proxy/src/headers.rs` | header 处理。 | 保留 provider request metadata。 | agent 原生元数据不能被压缩当文本。 |
| `crates/headroom-proxy/src/responses_items.rs` | Responses item helpers。 | OpenAI Responses item 格式处理。 | provider projection 必须懂目标格式。 |
| `crates/headroom-proxy/src/websocket.rs` | websocket。 | session/runtime 通道。 | 对实时 session registry 有参考。 |
| `crates/headroom-proxy/src/health.rs` | 健康检查。 | 间接。 | 非核心。 |

#### 19.5.1 compression 投影

| 文件 | 角色 | 关键设计 | memorph 学习点 |
| --- | --- | --- | --- |
| `compression/mod.rs` | compression 聚合。 | provider-specific compression 入口。 | 要有 projection 层，按 agent 输出压缩标记。 |
| `compression/model_limits.rs` | 模型限制。 | 上下文窗口、阈值、预算。 | 切换到目标 agent 时使用目标预算。 |
| `compression/anthropic.rs` | Anthropic 压缩。 | Anthropic message/cache control 格式。 | Claude 迁移时保留原生 compact/cache 语义。 |
| `compression/live_zone_anthropic.rs` | Anthropic live-zone 投影。 | core live-zone 结果映射到 Anthropic 请求。 | Claude exporter 应投影为 Claude 能理解且不误读的结构。 |
| `compression/live_zone_openai.rs` | OpenAI chat 投影。 | live-zone 结果映射到 Chat Completions。 | Codex/OpenAI 类 agent 要保留 compressed marker，不展开 archive。 |
| `compression/live_zone_responses.rs` | OpenAI Responses 投影。 | 处理 Responses API item。 | Codex 如果接近 Responses/Chat，必须按目标数据格式投影。 |

#### 19.5.2 handlers、cache、SSE、observability

| 文件 | 角色 | memorph 学习点 |
| --- | --- | --- |
| `handlers/mod.rs` | handler 聚合。 | agent adapter 不应互相调用。 |
| `handlers/chat_completions.rs` | OpenAI Chat handler。 | OpenAI/Codex projection 需要专门测试。 |
| `handlers/responses.rs` | OpenAI Responses handler。 | 不同 API item 对压缩 marker 的位置不同。 |
| `handlers/conversations.rs` | conversations handler。 | canonical session 可参考 conversation item 思路。 |
| `cache_stabilization/mod.rs` | cache 稳定模块聚合。 | “不要破坏 agent 原生 cache/compact”是投影质量标准。 |
| `cache_stabilization/anthropic_cache_control.rs` | Anthropic cache_control。 | Claude 压缩迁移时不能丢 cache control。 |
| `cache_stabilization/openai_cache_key.rs` | OpenAI cache key。 | Codex/OpenAI 字段顺序和稳定内容会影响 cache。 |
| `cache_stabilization/tool_def_normalize.rs` | tool 定义标准化。 | workspace/tool schema 不应每次切换都造成上下文抖动。 |
| `cache_stabilization/volatile_detector.rs` | 动态内容检测。 | 压缩和 cache 都要避开 volatile 噪声。 |
| `cache_stabilization/drift_detector.rs` | 漂移检测。 | 多次迁移/二次压缩需要检测语义漂移。 |
| `sse/framing.rs` | SSE frame 处理。 | 协议内容不能误当会话正文。 |
| `sse/openai_chat.rs` | OpenAI chat SSE。 | Codex/OpenAI streaming 解析参考。 |
| `sse/openai_responses.rs` | Responses SSE。 | Responses 增量需要专门处理。 |
| `sse/anthropic.rs` | Anthropic SSE。 | Claude streaming 解析参考。 |
| `observability/compression_ratio.rs` | 压缩率指标。 | manifest 应计算 before/after token/bytes。 |
| `observability/cache_hit_rate.rs` | cache 命中率。 | hot zone 稳定可以量化。 |
| `observability/proxy_metrics.rs` | proxy metrics。 | 请求级 compression outcome 可聚合。 |
| `observability/prometheus.rs`、`metric_names.rs` | Prometheus 指标。 | 当前不必实现，但指标命名要预留。 |

#### 19.5.3 Bedrock/Vertex/provider envelope

| 文件 | 角色 | memorph 相关性 |
| --- | --- | --- |
| `bedrock/mod.rs`、`vertex/mod.rs` | provider 模块聚合。 | provider envelope 应隔离在 adapter。 |
| `bedrock/envelope.rs`、`vertex/envelope.rs` | 请求/响应 envelope。 | 同一压缩结果需要不同外壳。 |
| `bedrock/invoke.rs`、`bedrock/invoke_streaming.rs` | Bedrock 调用。 | 压缩后请求仍需适配 provider 调用方式。 |
| `bedrock/eventstream.rs`、`eventstream_to_sse.rs` | eventstream/SSE 转换。 | provider output 转换不能丢元信息。 |
| `bedrock/sigv4.rs`、`auth_mode_layer.rs`、`vertex/adc.rs` | 认证。 | 非压缩核心，只证明 auth 与 compression 分层。 |
| `vertex/raw_predict.rs`、`stream_raw_predict.rs` | Vertex 调用。 | provider-specific 请求投影位置参考。 |

### 19.6 Python CCR/cache：旧实现中仍值得学习的部分

| 文件 | 角色 | 会话/压缩相关性 | memorph 学习点 |
| --- | --- | --- | --- |
| `headroom/ccr/tool_injection.py` | 将 retrieve 工具注入 agent。 | 让模型需要细节时主动取回。 | 当前不实现 MCP/tool injection，但学习“默认压缩、按需恢复”。 |
| `headroom/ccr/response_handler.py` | 处理 retrieve 响应。 | 将外部存储内容恢复给模型。 | `retrieve archive block` 应有统一响应格式。 |
| `headroom/ccr/context_tracker.py` | 跟踪上下文引用。 | 记录哪些内容被压缩、引用、恢复。 | 需要 archive lineage 和 session-level compression map。 |
| `headroom/ccr/batch_store.py` | 批量存储。 | 多块压缩内容持久化。 | 一次压缩通常产生多个 block，需要 batch manifest。 |
| `headroom/ccr/batch_processor.py` | 批量处理。 | 批量压缩/恢复处理。 | 主动压缩可先分析所有候选，再一次提交。 |
| `headroom/ccr/mcp_server.py` | MCP server。 | 暴露 retrieve 等工具。 | 不实现 MCP，只保留接口思想。 |
| `headroom/ccr/__init__.py` | 包导出。 | 旧 Python CCR 入口。 | 低价值。 |
| `headroom/cache/compression_store.py` | 压缩内容存储。 | 最接近 memorph archive。 | 学习“原文独立 store”，但要做 durable archive，不能只是缓存。 |
| `headroom/cache/compression_cache.py` | 压缩结果缓存。 | 避免重复压缩。 | 相同工具输出可复用压缩结果。 |
| `headroom/cache/compression_feedback.py` | 压缩反馈。 | 记录压缩效果。 | manifest 应反馈 ratio、quality、reject reason。 |
| `headroom/cache/dynamic_detector.py` | 动态内容检测。 | 识别 volatile 内容。 | 压缩候选和 cache key 都需要去噪。 |
| `headroom/cache/prefix_tracker.py` | prefix 跟踪。 | 跟踪稳定前缀。 | hot zone/workspace state 需要稳定边界。 |
| `headroom/cache/base.py`、`backends/base.py` | cache 基础接口。 | 定义 cache 后端。 | archive/cache 应分开：archive 是可恢复真源，cache 是加速。 |
| `headroom/cache/backends/memory.py` | 内存 cache。 | 测试/短生命周期。 | 不能作为会话压缩持久层。 |
| `headroom/cache/semantic.py` | semantic cache。 | 语义相似缓存。 | 后续可辅助 retrieve，不是第一阶段核心。 |
| `headroom/cache/anthropic.py`、`openai.py`、`google.py` | provider cache。 | provider-specific cache。 | projection 要尊重 provider cache 差异。 |
| `headroom/cache/registry.py` | cache registry。 | provider/cache 后端选择。 | 需要 agent capability registry。 |
| `headroom/cache/__init__.py`、`backends/__init__.py` | 包导出。 | 低。 | 不重点学习。 |

### 19.7 Python memory：长期记忆和会话压缩必须分清

Headroom 的 memory 模块很多，但它和“可逆会话压缩”不是一回事。memorph 可以学习 storage、ranking、injection、writer，但不能把压缩 archive 直接降级成长记忆摘要。

| 文件 | 角色 | 相关性 | memorph 取舍 |
| --- | --- | --- | --- |
| `headroom/memory/models.py` | memory 数据模型。 | 高。 | 可参考结构化 metadata，但 archive 要保存原文和 lineage。 |
| `headroom/memory/core.py` | memory 核心服务。 | 高。 | 可参考 add/search/update/delete 边界。 |
| `headroom/memory/storage_router.py` | 存储路由。 | 高。 | workspace/session/global 作用域路由有价值。 |
| `headroom/memory/budget.py` | memory 注入预算。 | 高。 | retrieve 时不能无限塞回上下文，要有预算。 |
| `headroom/memory/ports.py` | port/interface。 | 高。 | 应定义 archive/retrieval port。 |
| `headroom/memory/sync.py` | 同步。 | 高。 | 对跨 agent/session 同步有参考。 |
| `headroom/memory/bridge.py` | bridge。 | 高。 | 不同工具格式桥接，类似 agent migration。 |
| `headroom/memory/bridge_parsers.py` | bridge parser。 | 高。 | 需要 agent 原生 compact/summary marker parser。 |
| `headroom/memory/bridge_config.py` | bridge 配置。 | 中。 | provider bridge 配置参考。 |
| `headroom/memory/config.py`、`factory.py` | 配置和 factory。 | 中。 | archive/retrieval 配置可独立。 |
| `headroom/memory/extraction.py`、`inline_extractor.py` | 从文本抽取 memory。 | 中。 | 用于长期知识，不用于可逆压缩。 |
| `headroom/memory/tracker.py` | memory 使用跟踪。 | 中。 | 可参考 archive block 使用计数。 |
| `headroom/memory/system.py` | system memory。 | 中。 | 系统级背景和会话 archive 要分层。 |
| `headroom/memory/tools.py`、`wrapper.py`、`wrapper_tools.py` | 工具/wrapper。 | 中。 | retrieve 接口形状参考，不照搬工具注入。 |
| `headroom/memory/traffic_learner.py` | 从流量学习。 | 低到中。 | future，可从会话流量找压缩策略。 |
| `headroom/memory/easy.py` | 简化入口。 | 低。 | 对压缩核心价值小。 |
| `headroom/memory/qdrant_env.py` | Qdrant 环境。 | 低。 | 第一阶段不需要。 |
| `headroom/memory/mcp_server.py` | memory MCP。 | 中。 | 不实现 MCP。 |

#### 19.7.1 memory adapters/backends/writers

| 文件 | 角色 | memorph 取舍 |
| --- | --- | --- |
| `memory/adapters/sqlite.py` | SQLite 基础存储。 | archive 第一阶段可选。 |
| `memory/adapters/fts5.py` | SQLite FTS5。 | 非 embedding 检索非常适合 archive retrieve。 |
| `memory/adapters/sqlite_vector.py`、`hnsw.py`、`embedders.py` | 向量和 embedding。 | 第二阶段可选，不作为硬依赖。 |
| `memory/adapters/cache.py` | memory cache。 | 可加速 retrieve。 |
| `memory/adapters/graph.py`、`graph_models.py`、`sqlite_graph.py` | graph。 | 未来 session lineage graph 可参考。 |
| `memory/backends/local.py` | 本地 backend。 | archive 最现实的落点。 |
| `memory/backends/mem0.py`、`direct_mem0.py`、`mem0_system_adapter.py` | mem0 后端。 | 不照搬，避免外部 memory 系统模糊 archive 边界。 |
| `memory/sync_adapters/claude_code.py` | Claude Code memory 同步。 | Claude 原生文件/会话结构要专门理解。 |
| `memory/sync_adapters/codex_agent.py` | Codex agent memory 同步。 | Codex 会话/记忆格式要单独 adapter。 |
| `memory/writers/base.py` | writer 抽象。 | provider exporter 应有统一 writer。 |
| `memory/writers/claude_writer.py` | Claude writer。 | Claude projection 参考。 |
| `memory/writers/codex_writer.py` | Codex writer。 | Codex projection 参考。 |
| `memory/writers/cursor_writer.py`、`generic_writer.py` | 其他 writer。 | 扩展参考。 |
| `memory/adapters/__init__.py`、`backends/__init__.py`、`sync_adapters/__init__.py`、`writers/__init__.py` | 包导出。 | 低价值。 |

### 19.8 Python proxy：旧请求路径中的策略碎片

这些文件说明旧 Python 体系里 compression/memory/cache/provider 的很多策略分散在 proxy。memorph 不能复制这种散落方式，应把策略收回到独立 compression/session 模块。

| 文件 | 角色 | 会话/压缩相关性 | memorph 取舍 |
| --- | --- | --- | --- |
| `headroom/proxy/server.py` | Python proxy server。 | 请求入口，串联 handler、compression、memory。 | 不照搬 server，但学习请求前统一处理位置。 |
| `headroom/proxy/models.py` | proxy 模型。 | 请求/响应结构。 | provider projection 需要强类型模型。 |
| `headroom/proxy/compression_decision.py` | 文本压缩决策。 | 判断是否压缩。 | 应保留独立 decision，但移到 core。 |
| `headroom/proxy/image_compression_decision.py` | 图像压缩决策。 | 非文本。 | 当前不纳入。 |
| `headroom/proxy/memory_decision.py` | memory 注入决策。 | 判断是否注入 memory。 | 与 archive retrieve 类似，但语义不同。 |
| `headroom/proxy/memory_handler.py` | memory 处理。 | 执行检索/注入。 | 可学习 budgeted injection。 |
| `headroom/proxy/memory_injection.py` | 注入格式。 | 将 memory 写入请求。 | projection 要定义压缩块怎样呈现给目标 agent。 |
| `headroom/proxy/memory_query.py` | memory 查询。 | query 构造。 | archive retrieve 需要 query builder。 |
| `headroom/proxy/memory_ranker.py` | memory 排序。 | 相关性排序。 | 压缩块恢复不能无条件全展开。 |
| `headroom/proxy/memory_tool_adapter.py` | memory tool adapter。 | 工具化接口。 | 当前不实现 MCP/tool，但接口参考有价值。 |
| `headroom/proxy/semantic_cache.py` | 语义 cache。 | 请求级缓存。 | 非压缩核心。 |
| `headroom/proxy/savings_tracker.py` | 节省统计。 | token/cost savings。 | manifest/metrics 可学习。 |
| `headroom/proxy/cost.py` | 成本估算。 | token/cost 转换。 | 可先只记录 token，不做价格。 |
| `headroom/proxy/outcome.py` | outcome 类型。 | 记录处理结果。 | compression outcome 必须显式。 |
| `headroom/proxy/debug_introspection.py` | debug introspection。 | 排查压缩/注入路径。 | 需要 `inspect compression` 能力。 |
| `headroom/proxy/prometheus_metrics.py` | metrics。 | 观测。 | future。 |
| `headroom/proxy/request_logger.py` | 请求日志。 | 审计。 | 压缩流程可审计，但注意隐私。 |
| `headroom/proxy/stage_timer.py` | 阶段耗时。 | pipeline 性能。 | 主动压缩可记录耗时。 |
| `headroom/proxy/rate_limiter.py` | 限流。 | LLM/remote provider 相关。 | 若引入 LLM 压缩才需要。 |
| `headroom/proxy/ws_session_registry.py` | websocket session registry。 | 管理运行时 session。 | workspace/session 一等能力可参考。 |
| `headroom/proxy/auth_mode.py`、`modes.py`、`extensions.py` | 模式和扩展。 | 行为切换。 | 可有 compression mode：off/passive/active/user-selected。 |
| `headroom/proxy/loopback_guard.py`、`warmup.py`、`helpers.py` | 辅助。 | 低。 | 不重点学习。 |
| `headroom/proxy/handlers/openai.py`、`anthropic.py`、`gemini.py` | provider handler。 | provider-specific 请求处理。 | Codex/Claude/Kimi/DeepSeek adapter 需要分别测试。 |
| `headroom/proxy/handlers/batch.py` | batch handler。 | 批处理请求。 | 批量压缩候选参考。 |
| `headroom/proxy/handlers/streaming.py` | streaming handler。 | 流式响应。 | 流式会话记录时要避免碎片化。 |
| `headroom/proxy/interceptors/base.py`、`astgrep.py` | interceptor。 | 请求/内容拦截、代码结构识别。 | 可有 pre-project interceptor，但不要滥用。 |
| `headroom/proxy/__init__.py`、`handlers/__init__.py`、`interceptors/__init__.py` | 包导出。 | 低。 | 不重点学习。 |

### 19.9 Python providers、learn、relevance

#### 19.9.1 providers

| 文件 | 角色 | 会话/压缩相关性 | memorph 学习点 |
| --- | --- | --- | --- |
| `headroom/providers/base.py` | provider 基类。 | provider 能力抽象。 | 需要 agent capability trait：支持 native compact、marker、archive ref。 |
| `headroom/providers/registry.py` | provider registry。 | provider 查找。 | 新增 agent 不改 core。 |
| `headroom/providers/proxy_routes.py` | provider 路由。 | endpoint 映射。 | importer/exporter 路由参考。 |
| `headroom/providers/openai.py`、`openai_compatible.py` | OpenAI 类 provider。 | OpenAI envelope。 | DeepSeek 等即使兼容 API，compact 语义也未必兼容。 |
| `headroom/providers/anthropic.py` | Anthropic provider。 | Claude envelope。 | Claude 原生压缩/缓存标记要单独处理。 |
| `headroom/providers/google.py`、`cohere.py`、`litellm.py` | 其他 provider。 | 多 provider 差异。 | 统一 API 不代表统一压缩语义。 |
| `headroom/providers/claude/runtime.py` | Claude runtime wrapper。 | agent runtime 接入。 | Claude 会话格式和 native compact 需要 adapter。 |
| `headroom/providers/codex/runtime.py` | Codex runtime wrapper。 | Codex runtime 接入。 | 对 Codex 会话导出/识别最相关。 |
| `headroom/providers/cursor/runtime.py`、`gemini/runtime.py`、`aider/runtime.py` | 其他 runtime。 | agent runtime 接入。 | 多 agent 扩展参考。 |
| `headroom/providers/copilot/wrap.py`、`openclaw/wrap.py` | wrapper。 | agent 包装。 | wrapper 形态参考。 |
| `headroom/providers/*/install.py`、`install_registry.py` | 安装。 | 与压缩语义弱。 | 不学压缩。 |
| `headroom/providers/*/__init__.py` | 包导出。 | 低。 | 不重点学习。 |

#### 19.9.2 learn

| 文件 | 角色 | 会话/压缩相关性 | memorph 学习点 |
| --- | --- | --- | --- |
| `headroom/learn/models.py` | 学习模型。 | 扫描/学习结果。 | 可参考 agent capability discovery 数据结构。 |
| `headroom/learn/base.py` | plugin 基类。 | 不同 agent 学习插件。 | 新 agent 支持可走 plugin。 |
| `headroom/learn/registry.py` | plugin registry。 | 注册 Claude/Codex/Gemini。 | 新增 agent 不改 core。 |
| `headroom/learn/scanner.py` | 扫描器。 | 扫描本地 agent 配置/会话。 | 可扫描 agent 原生会话目录。 |
| `headroom/learn/analyzer.py` | 分析器。 | 分析 agent 使用模式。 | 可识别压缩 marker 和 session 文件。 |
| `headroom/learn/writer.py` | 写回学习结果。 | 写配置/结果。 | 可写 agent profile。 |
| `headroom/learn/plugins/claude.py` | Claude 学习插件。 | 识别 Claude 环境。 | Claude adapter discovery 参考。 |
| `headroom/learn/plugins/codex.py` | Codex 学习插件。 | 识别 Codex 环境。 | Codex 会话数据理解参考。 |
| `headroom/learn/plugins/gemini.py` | Gemini 学习插件。 | 识别 Gemini 环境。 | 多 agent 扩展参考。 |
| `headroom/learn/_shared.py`、`__init__.py`、`plugins/__init__.py` | 辅助/导出。 | 低。 | 不重点学习。 |

#### 19.9.3 Python relevance

| 文件 | 角色 | 会话/压缩相关性 | memorph 学习点 |
| --- | --- | --- | --- |
| `headroom/relevance/base.py` | relevance 接口。 | archive retrieve 的抽象基础。 | retrieve 不应写死 BM25 或 embedding。 |
| `headroom/relevance/bm25.py` | BM25 检索。 | 无外部 LLM/embedding 的检索。 | 第一阶段推荐实现。 |
| `headroom/relevance/embedding.py` | embedding 检索。 | 语义召回。 | 第二阶段可选。 |
| `headroom/relevance/hybrid.py` | hybrid 检索。 | 混合排序。 | 适合质量提升。 |
| `headroom/relevance/__init__.py` | 导出。 | 低。 | 不重点学习。 |

### 19.10 Rust tests：Headroom 明确锁定的行为

| 文件 | 测试目标 | memorph 应有对应测试 |
| --- | --- | --- |
| `crates/headroom-core/tests/ccr_roundtrip.rs` | CCR 存储和恢复 roundtrip。 | archive 原文保存、压缩引用、恢复内容一致。 |
| `crates/headroom-core/tests/ccr_backends.rs` | 多 backend 行为一致。 | file/sqlite/in-memory archive 行为一致。 |
| `crates/headroom-core/tests/live_zone_ccr.rs` | live-zone 与 CCR 联动。 | 压缩候选转 archive ref 后 provider projection 正确。 |
| `crates/headroom-core/tests/live_zone_dispatch.rs` | live-zone dispatch。 | 不同 content type 走不同 compressor。 |
| `crates/headroom-core/tests/live_zone_thresholds.rs` | 阈值控制。 | 小内容不压、收益不足不压、热区不压。 |
| `crates/headroom-core/tests/live_zone_token_validation.rs` | token validation。 | 压缩后 token 不下降则 reject。 |
| `crates/headroom-core/tests/tokenizer_proptest.rs` | tokenizer 性质测试。 | token 估算不崩、不负数、跨 provider 稳定。 |
| `crates/headroom-core/tests/cache_control.rs` | cache control 行为。 | provider compact/cache marker 不丢。 |
| `crates/headroom-core/tests/recommendations_loader.rs` | recommendation 加载。 | policy 配置加载测试。 |
| `crates/headroom-core/tests/auth_mode.rs` | auth mode。 | memorph 不需要同类压缩测试。 |
| `crates/headroom-proxy/tests/integration_compression.rs` | proxy compression 集成。 | canonical session -> target agent projection -> compressed marker。 |
| `crates/headroom-proxy/tests/integration_cache_control.rs`、`integration_cache_drift.rs` | cache control 和漂移。 | 多次迁移/二次压缩不应语义漂移。 |
| `crates/headroom-proxy/tests/integration_e3_anthropic_cache_control.rs` | Anthropic cache。 | Claude 兼容测试。 |
| `crates/headroom-proxy/tests/integration_e4_openai_cache_key.rs` | OpenAI cache key。 | Codex/OpenAI 兼容测试。 |
| `crates/headroom-proxy/tests/integration_chat_completions.rs`、`integration_responses.rs`、`integration_responses_streaming.rs` | OpenAI Chat/Responses。 | OpenAI/Codex 投影和 streaming 测试。 |
| `crates/headroom-proxy/tests/integration_conversations.rs` | conversations。 | 多轮会话结构测试。 |
| `crates/headroom-proxy/tests/integration_bedrock_*`、`integration_vertex_raw_predict.rs` | provider adapter。 | provider adapter 独立测试，不污染 core。 |
| `crates/headroom-proxy/tests/integration_sse.rs`、`sse_openai_chat.rs`、`sse_openai_responses.rs`、`sse_anthropic.rs`、`sse_framing.rs` | SSE。 | 流式输出解析不应污染压缩。 |
| `crates/headroom-proxy/tests/integration_schema_sort.rs`、`integration_tool_sort.rs` | schema/tool 稳定。 | workspace/tool definition 稳定性。 |
| `crates/headroom-proxy/tests/integration_volatile_detector.rs` | volatile 检测。 | 压缩/cache 前去噪。 |
| `crates/headroom-proxy/tests/integration_metrics.rs`、`integration_bedrock_metrics.rs` | metrics。 | compression manifest 可统计。 |
| `crates/headroom-proxy/tests/integration_headers.rs`、`integration_request_id.rs` | metadata/trace。 | trace id/archive id 关联。 |
| `crates/headroom-proxy/tests/integration_body.rs`、`integration_body_size.rs` | 请求体处理。 | 压缩前后大小约束。 |
| `crates/headroom-proxy/tests/common/mod.rs` | 测试公共工具。 | 应建设 fixture helpers。 |
| `crates/headroom-proxy/tests/e2e_real.rs` | 真实 e2e。 | 最终需要真实 agent fixture，但 CI 可分层。 |
| `crates/headroom-proxy/tests/integration_http.rs`、`integration_health.rs`、`integration_ws.rs` | HTTP/health/ws。 | memorph 不需要 HTTP，但需要 pipeline smoke。 |

### 19.11 从逐文件索引反推出 memorph 的模块边界

| memorph 板块 | 对应 Headroom 文件群 | 职责 | 不应该做什么 |
| --- | --- | --- | --- |
| `compression-core` | `crates/headroom-core/src/transforms/*` | 类型检测、候选选择、压缩执行、token validation、manifest。 | 不知道 Claude/Codex/Kimi 的文件格式。 |
| `compression-archive` | `ccr/*`、`cache/compression_store.py`、`ccr/backends/*` | 保存原文、压缩结果、lineage、retrieval index。 | 不把 archive 当临时 cache。 |
| `compression-retrieval` | `relevance/*`、`memory_query.py`、`memory_ranker.py` | query-first retrieve。 | 不无条件展开所有压缩块。 |
| `provider-projection` | `crates/headroom-proxy/src/compression/*`、`providers/*` | 把 canonical 压缩块映射成目标 agent 能理解的格式。 | 不重新决定压缩策略。 |
| `agent-parser` | `learn/plugins/*`、`memory/bridge_parsers.py` | 识别源 agent 的原生 compact/summary/cache marker。 | 不把未知 marker 当普通文本。 |
| `workspace/session-registry` | `ws_session_registry.py`、`memory/storage_router.py` | workspace 作用域、session 作用域、agent profile。 | 不把所有压缩块混到全局。 |
| `observability` | `observability/*`、`savings_tracker.py`、`outcome.py` | 记录 ratio、reason、trace、失败。 | 不只在日志里打印一句 `compressed`。 |
| `tests/fixtures` | `tests/live_zone_*`、`tests/integration_compression.rs` | 锁定跨 agent 压缩行为。 | 不只测 happy path。 |

### 19.12 对 memorph 当前目标的直接落地清单

| 能力 | 必要性 | Headroom 证据 | memorph 实现标准 |
| --- | --- | --- | --- |
| canonical compressed block | 必须 | `ccr/mod.rs`、`live_zone.rs` | 中间态有明确 `compressed_block` event，不是普通 assistant 文本。 |
| durable archive | 必须 | `ccr/backends/sqlite.rs`、`compression_store.py` | 原文落盘，可按 archive id 恢复。 |
| provider-native marker parser | 必须 | `memory/bridge_parsers.py`、`providers/codex/runtime.py`、`providers/claude/runtime.py` | Claude/Codex/Kimi/DeepSeek 的压缩标记要识别并映射。 |
| provider projection | 必须 | `live_zone_anthropic.rs`、`live_zone_openai.rs`、`live_zone_responses.rs` | A 中压缩的内容到 B 中不能无条件展开，而要按 B 的压缩语义投影。 |
| token validation | 必须 | `live_zone_token_validation.rs` | 压缩后目标上下文不变小则 reject。 |
| hot zone protection | 必须 | `04-phase-B-live-zone.md`、`live_zone.rs` | 当前任务目标、最新问题、workspace state 不压。 |
| type-aware non-LLM compression | 强烈建议 | `log_compressor.rs`、`search_compressor.rs`、`diff_compressor.rs`、`smart_crusher/*` | 先压日志/搜索/diff/JSON，不先做泛化模型摘要。 |
| query-first retrieve | 必须 | `ccr/tool_injection.py`、`relevance/*` | 目标 agent 只在需要时取回片段。 |
| manifest/outcome | 必须 | `outcome.py`、`observability/compression_ratio.rs` | 每次压缩记录候选、策略、ratio、skip reason、archive id。 |
| user-selected compression | 建议 | `planning.rs`、`batch_processor.py` | 用户可选择候选；未选内容保留。 |
| LLM compression | 后置 | Headroom 主线强调 deterministic/smart crusher | 只用于难以结构化压缩的自然语言长段，并且必须保留原文 archive。 |

### 19.13 哪些 Headroom 文件不应该成为 memorph 的实现目标

| 文件群 | 不纳入原因 |
| --- | --- |
| `headroom/ccr/mcp_server.py`、`headroom/memory/mcp_server.py`、`docs/content/docs/mcp.mdx` | 用户已明确不实现 MCP。只借鉴 retrieve 接口理念。 |
| `headroom/memory/backends/mem0*.py` | 引入外部 memory 系统会模糊 archive 和 memory 边界。 |
| `qdrant_env.py`、embedding/vector 相关复杂后端 | 第一阶段会增加部署复杂度，BM25/FTS 足够起步。 |
| `image_compression_decision.py`、`image-compression.mdx` | 当前任务是会话文本压缩。 |
| provider install 文件 | 安装包装与压缩语义无关。 |
| Bedrock/Vertex 认证细节 | 证明 provider adapter 要隔离即可，不需要复制。 |
| Python proxy server 整体 | Headroom 自己也在 realignment 中转向 Rust core/proxy，不应照搬旧散乱架构。 |

### 19.14 最终文件级结论

从文件级调研看，Headroom 对 memorph 最有价值的不是某一个 Python 类，也不是 MCP，而是下面这条完整链路：

```text
detect content type
-> choose compression candidate
-> protect hot zone and provider markers
-> apply deterministic/type-aware compression first
-> store original content in durable archive
-> replace middle-state payload with compressed block ref
-> validate token/byte savings
-> record manifest/outcome
-> project compressed semantics into target agent format
-> retrieve by query only when needed
```

memorph 的关键优势是它已经处在“跨 agent 会话中间态”的位置，所以它比 Headroom 的 HTTP proxy 更适合做会话压缩管理。Headroom 是请求前压缩，memorph 应该是会话中间态压缩和跨 agent projection；两者理念相同，但落点不同。

因此，后续 memorph 不应该做一个“模型总结按钮”，而应该做一个独立、可审计、可恢复、provider-aware 的 `SessionCompressionManager`。这个模块的验收标准不是“看起来短了”，而是：

- 中间态物理变小。
- 原文可按 archive id 恢复。
- 目标 agent 不把已压缩垃圾当完整上下文读。
- 不同 agent 的原生压缩标记能被解析和投影。
- 压缩收益可量化。
- 压缩失败可解释。
- 当前任务目标和 workspace 状态不丢。
