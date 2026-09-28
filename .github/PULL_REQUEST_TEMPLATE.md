<!-- 标题格式：类别：做了什么 -->

## 这个 PR 做了什么

<!-- 一两句话说明改了什么、为什么 -->

## 检查项

- [ ] 三个 crate 都能 `cargo build`（根目录、`gui/`、`server/`）
- [ ] `cargo test` 通过（48 项），`cd server && cargo test` 通过
- [ ] `cargo clippy --all-targets` 无新增警告
- [ ] 已执行 `cargo fmt`
- [ ] 行为有变化时，已更新 `docs/design.md`（必要时含 `API.md`、`README.md`）
