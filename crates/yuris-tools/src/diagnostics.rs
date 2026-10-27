//! Scoped JSONL tracing export, active before VFS and VM initialization.
use std::{fs::File, io::Write, sync::{Arc, Mutex}};
use tracing_subscriber::prelude::*;

#[derive(Clone)]
struct JsonLog {
    file: Arc<Mutex<File>>,
    error: Arc<Mutex<Option<std::io::Error>>>,
}
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for JsonLog {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if !event.metadata().target().starts_with("yuris") { return; }
        struct Fields(serde_json::Map<String, serde_json::Value>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().into(), serde_json::Value::String(format!("{value:?}")));
            }
        }
        let mut fields = Fields(Default::default());
        event.record(&mut fields);
        let record = serde_json::json!({
            "type": "diagnostic", "schema_version": 1,
            "level": event.metadata().level().as_str(),
            "target": event.metadata().target(), "fields": fields.0,
        });
        let result = writeln!(self.file.lock().unwrap(), "{record}");
        if let Err(e) = result { *self.error.lock().unwrap() = Some(e); }
    }
}

pub(crate) fn capture<T>(file: File, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let layer = JsonLog { file: Arc::new(Mutex::new(file)), error: Arc::new(Mutex::new(None)) };
    let errors = layer.error.clone();
    let subscriber = tracing_subscriber::registry().with(layer);
    let result = tracing::subscriber::with_default(subscriber, f);
    if let Some(e) = errors.lock().unwrap().take() {
        return Err(anyhow::anyhow!("diagnostic export failed: {e}; operation error: {:?}", result.as_ref().err()));
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn exports_diagnostics_even_when_operation_fails() {
        let path = std::env::temp_dir().join(format!("yuris-diagnostics-{}.jsonl", std::process::id()));
        let dir = std::env::temp_dir().join(format!("yuris-diagnostic-game-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ysbin")).unwrap();
        let mut table = b"YSVR".to_vec();
        table.extend(555u32.to_le_bytes());
        table.extend(0u16.to_le_bytes());
        std::fs::write(dir.join("ysbin/ysv.ybn"), table).unwrap();
        let result: anyhow::Result<()> = super::capture(std::fs::File::create(&path).unwrap(), || {
            yuris_player::Player::open(&yuris_player::Config { game_dir: dir.clone(), ..Default::default() })
                .map(|_| ()).map_err(Into::into)
        });
        assert!(result.is_err());
        let data = std::fs::read_to_string(&path).unwrap();
        let rows: Vec<serde_json::Value> = data.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert!(rows.iter().any(|r| r["target"] == "yuris_version"));
        assert!(rows.iter().any(|r| r["target"] == "yuris_boot" && r["level"] == "ERROR"));
        assert!(data.contains("layout_status=ambiguous"));
        assert!(data.contains("reason=ysvr_layout_unresolved"));
        assert!(data.contains("boot_failed stage=script_key"));
        assert!(data.contains("loose:ysbin"));
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
