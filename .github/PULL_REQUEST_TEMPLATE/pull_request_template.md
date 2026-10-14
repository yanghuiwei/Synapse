---
name:  pull_request
about: 提交代码改动
title: 'feat(scope): '
labels: ''
body:
  - type: markdown
    attributes:
      value: |
        感谢你的贡献。提交前请确认已读过 [贡献指南](../CONTRIBUTING.md)。

  - type: textarea
    id: what
    attributes:
      label: 改了什么
      description: 客观陈述改动的范围。
    validations:
      required: true

  - type: textarea
    id: why
    attributes:
      label: 为什么这么改
      description: |
        这条最重要。说明问题是什么、你为什么选了���个方案、
        考虑过并否决了哪些做法。

        如果改动的起因是一个 bug，请额外说明**它是怎么被发现的**
        —— 这决定了后来的人怎么判断它是否已被彻底解决。
    validations:
      required: true

  - type: textarea
    id: verify
    attributes:
      label: 怎么验证的
      description: 写清楚你实际跑了什么、看到了什么结果。「已测试」不算验证。
      placeholder: |
        cargo test --manifest-path src-tauri/Cargo.toml -p reqly-core
        → 515 passed / 0 failed
        手动验证：新建请求 → 保存 → 切换集合 → 再切回来，请求仍在。
    validations:
      required: true

  - type: textarea
    id: checklist
    attributes:
      label: 自查清单
      value: |
        - [ ] 提交前跑过 `cargo fmt` 与 `cargo clippy -- -D warnings`
        - [ ] 补了测试，或说明了为什么这次不需要
        - [ ] 没改版本号与 `CHANGELOG.md`
        - [ ] 注释解释的是「为什么」，不是复述「做了什么」
        - [ ] 如果碰了依赖，没引入只在 Windows 上成立的平台相关依赖
        - [ ] 没有真实凭据、内部域名或个人可识别信息
