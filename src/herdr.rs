//! herdr の socket API クライアント。
//!
//! `$HERDR_SOCKET_PATH` の Unix domain socket に JSON Lines で話す。
//! リクエストの応答と購読イベントが同じ接続に混ざって流れてくる。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> Result<Self, Box<dyn std::error::Error>> {
        let path = std::env::var("HERDR_SOCKET_PATH")
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                format!("{home}/.config/herdr/herdr.sock")
            });
        let stream = UnixStream::connect(&path)
            .map_err(|e| format!("herdr socket {path} に接続できません: {e}"))?;
        Ok(Client {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        })
    }

    fn send(&mut self, req: Value) -> Result<(), Box<dyn std::error::Error>> {
        self.writer.write_all(req.to_string().as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    /// 次の 1 行を読む。応答もイベントも区別せずに返す。
    pub fn next(&mut self) -> Result<Option<Value>, Box<dyn std::error::Error>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            return Ok(Some(Value::Null));
        }
        Ok(Some(serde_json::from_str(&line)?))
    }

    /// リクエストを投げ、同じ id の応答が来るまで読み飛ばす。
    /// 途中で流れてきたイベントは捨てずに返す。
    pub fn request(
        &mut self,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<(Value, Vec<Value>), Box<dyn std::error::Error>> {
        self.send(json!({"id": id, "method": method, "params": params}))?;
        let mut stray = Vec::new();
        loop {
            let Some(msg) = self.next()? else {
                return Err("herdr との接続が閉じました".into());
            };
            if msg.get("id").and_then(|v| v.as_str()) == Some(id) {
                if let Some(err) = msg.get("error") {
                    return Err(format!("{method} が失敗しました: {err}").into());
                }
                return Ok((msg["result"].clone(), stray));
            }
            if msg.get("event").is_some() {
                stray.push(msg);
            }
        }
    }

    pub fn subscribe(&mut self, kinds: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let subs: Vec<Value> = kinds.iter().map(|k| json!({"type": k})).collect();
        self.request("sub", "events.subscribe", json!({"subscriptions": subs}))?;
        Ok(())
    }
}

/// エージェントの状態。herdr の AgentStatus と対応する。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Blocked,
    Working,
    Done,
    Idle,
    Unknown,
}

impl Status {
    pub fn parse(s: &str) -> Status {
        match s {
            "blocked" => Status::Blocked,
            "working" => Status::Working,
            "done" => Status::Done,
            "idle" => Status::Idle,
            _ => Status::Unknown,
        }
    }

    /// ワークスペース内に複数エージェントがいるときの優先順位。
    /// 手が要るものほど強い。
    fn rank(self) -> u8 {
        match self {
            Status::Blocked => 4,
            Status::Done => 3,
            Status::Working => 2,
            Status::Idle => 1,
            Status::Unknown => 0,
        }
    }

    pub fn max(self, other: Status) -> Status {
        if other.rank() > self.rank() { other } else { self }
    }
}
