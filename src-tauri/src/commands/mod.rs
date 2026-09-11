//! Tauri 命令层。
//!
//! 每个命令只做三件事：**校验入参 → 调仓储 → 映射错误**。
//! 不写业务逻辑（那属于 repo / http / template 等模块），
//! 也不产生面向用户的文案（见 `crate::error` 的契约说明）。

pub mod folder;
pub mod request;
pub mod settings;
pub mod workspace;
