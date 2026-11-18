use std::fmt;

use anyhow::{Result, ensure};
use edikt_yaml::{Document, Expr, Step};
use serde::de::{self, DeserializeSeed, Visitor};
use serde_json::{Map, Number, Value};

#[derive(Clone, Copy)]
pub(super) enum Format {
    Yaml,
    Json5,
}

impl Format {
    pub(super) fn read(self, text: &str) -> Result<Value> {
        match self {
            Self::Yaml => serde_saphyr::from_str(text)
                .map_err(|_| anyhow::anyhow!("YAML 无效、含重复键或不是单份配置")),
            Self::Json5 => json5::from_str::<Json5Value>(text)
                .map(|value| value.0)
                .map_err(|_| anyhow::anyhow!("JSON5 无效、含重复键，或数值超出范围")),
        }
    }

    pub(super) fn patch(self, source: &str, expected: &Value) -> Result<String> {
        let original = self.read(source)?;
        if &original == expected {
            return Ok(source.to_owned());
        }
        let mut document: Box<dyn Document> = match self {
            Self::Yaml => Box::new(
                edikt_yaml::parse(source).map_err(|_| anyhow::anyhow!("无法编辑这份 YAML"))?,
            ),
            Self::Json5 => Box::new(
                edikt_jsonc::parse(source).map_err(|_| anyhow::anyhow!("无法编辑这份 JSON5"))?,
            ),
        };
        apply_changes(
            document.as_mut(),
            &mut Vec::new(),
            Some(&original),
            Some(expected),
        )?;
        let result = document.to_source();
        ensure!(
            self.read(&result)? == *expected,
            "此改动会影响其他字段或改变值的类型，原文件未改动"
        );
        Ok(result)
    }
}

fn apply_changes(
    doc: &mut dyn Document,
    path: &mut Vec<Step>,
    old: Option<&Value>,
    new: Option<&Value>,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    if let (Some(Value::Object(old)), Some(Value::Object(new))) = (old, new) {
        for key in old
            .keys()
            .chain(new.keys().filter(|key| !old.contains_key(*key)))
        {
            path.push(Step::Field(key.clone()));
            apply_changes(doc, path, old.get(key), new.get(key))?;
            path.pop();
        }
    } else {
        let path = Expr::Path(path.clone());
        let change = match new {
            Some(value) => Expr::Assign(
                Box::new(path),
                Box::new(Expr::Literal(
                    edikt_jsonc::parse(&value.to_string())?.to_value(),
                )),
            ),
            None => Expr::Call("del".into(), vec![path]),
        };
        doc.apply(&change)
            .map_err(|_| anyhow::anyhow!("无法单独修改此字段，原文件未改动"))?;
    }
    Ok(())
}

struct Json5Value(Value);

impl<'de> serde::Deserialize<'de> for Json5Value {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Json5Node(0).deserialize(deserializer).map(Self)
    }
}

#[derive(Clone, Copy)]
struct Json5Node(usize);

impl<'de> DeserializeSeed<'de> for Json5Node {
    type Value = Value;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        if self.0 >= 128 {
            return Err(de::Error::custom("too many nested values"));
        }
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Json5Node {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON5 configuration value")
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }

    fn visit_map<A: de::MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom("duplicate key"));
            }
            object.insert(key, entries.next_value_seed(Self(self.0 + 1))?);
        }
        Ok(Value::Object(object))
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let mut array = Vec::new();
        while let Some(value) = entries.next_element_seed(Self(self.0 + 1))? {
            array.push(value);
        }
        Ok(Value::Array(array))
    }
}
