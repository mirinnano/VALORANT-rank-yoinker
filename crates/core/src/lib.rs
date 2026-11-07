//! VALORANT の対戦情報を取得し、UI に渡してよい形へ変換するコアライブラリ。
//!
//! 外部から得たデータは [`source::RiotSource`] で受け取り、[`roster`] で
//! [`model::MatchSnapshot`] に変換する。UI が受け取るのはスナップショットだけで、
//! Streamer Mode のプレイヤーの本名は取得自体を行わない。

pub mod anonymize;
pub mod content;
pub mod demo;
pub mod diagnostics;
pub mod engine;
pub mod error;
pub mod images;
pub mod live;
pub mod local;
pub mod model;
pub mod presence;
pub mod profile;
pub mod rank;
pub mod replay;
pub mod retry;
pub mod review;
pub mod roster;
pub mod source;
pub mod stats;
pub mod store;
