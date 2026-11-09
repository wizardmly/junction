---
id: reformat-second-pass-stability
title: Reformat Code 第二遍不变（Kotlin 默认、Java Binary / Ternary）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/reformat-second-pass-stability
pr: 271
claimed: 2026-11-09
files: src/domain/reformat.rs
---

## 要做什么

格式化过的代码再按同样设置格式化一次应该一个字都不变（IntelliJ 就是这样）。现在不是：

- Signal Kotlin 语料（200 个文件，默认设置）：第一遍的结果再跑一遍，71 个文件约 1880 行还会变。多是多行实参 / lambda 收尾的续行缩进（`testDispatcher` + `) {` 之后的体多缩进一级，`group.recipientId` + `)` 多两格）。
- Signal Java 语料（400 个文件）：默认设置 0 行；Binary expressions / Ternary operation 打开（Chop 或 Wrap if long 都一样）时 35 行会变，链式调用 `.isCallFull(false)` 和 `||` 续行的缩进第二遍又加深。注解、枚举常量 Wrap always 时 0 行。

另外 `formatted_sources_are_stable`（`#[ignore]` 的语料测试）名字叫 stable，其实只把结果写到 `JUNCTION_FMT_OUT`，不比第二遍。改成也跑第二遍并报告（或断言）有变化的行，这样以后的格式化改动能直接看到。

做法：先给测试加第二遍比较，再挑最常见的几类不稳定按 IntelliJ 的缩进规则修，Kotlin 改动行数不能变多。

## 进度

- 2026-11-09：做完。
  - 原因：`chop_long_lines` 在缩进和空行都排完之后才拆长行，拆出的续行（`val x =` + `registerForActivityResult(…) { result ->`、`fun f() =` + `runTest {`）后面的体还是按原来那行缩进的；拆成多行的声明（Kotlin 里多行成员前后要空行）也没补空行。第二遍才按新续行排。
  - 修法：`reformat_with` 拆成 `reformat_pass(…, nested)`；拆行加了行时，把结果再走一遍（只多走一遍，`nested` 防止循环），选区按加的行数往后延。只在真拆了行时多花一遍的时间。
  - 语料测试 `formatted_sources_are_stable` 现在对每个文件再格式化一遍，有变化就打印 `UNSTABLE <文件> <行数>` 和前几行，最后断言 0。
  - 回归：Signal Kotlin 第二遍 71 个文件 → 0；Java 默认、Binary / Ternary 全开（Chop / 行尾符号和 Wrap if long / 下一行符号）都是 0。Kotlin 改动行数 23686 → 24934：多出的正好就是原来第二遍才出的那些（新的第一遍结果和原来第二遍的结果逐字节相同），都是 lambda / 块体跟着拆出的续行多缩进一级，和 IntelliJ 一样。Java 15746 不变。
  - 单元测试 `lays_out_again_after_chopping`（去掉修法时失败）。
