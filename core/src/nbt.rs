//! 最小可用的 NBT（Named Binary Tag）读写。
//!
//! 启动器只需要读/改 `level.dat`，但存档里的标签类型很多（列表、数组、嵌套
//! 复合标签），因此实现一个保留类型信息的通用树，避免“读得懂 A 存档、改坏 B 存档”。
//!
//! 格式参考：<https://minecraft.wiki/w/NBT_format>
//! `level.dat` 使用 gzip 压缩、根标签为 Compound。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_BYTE_ARRAY: u8 = 7;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

/// 防御损坏/恶意存档：限制列表长度与嵌套深度。
const MAX_LIST_LEN: usize = 8_000_000;
const MAX_DEPTH: usize = 512;
const MAX_NBT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TAGS: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    /// `element_type` 保留空列表的元素类型（NBT 要求）
    List {
        element_type: u8,
        items: Vec<Tag>,
    },
    Compound(BTreeMap<String, Tag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn type_id(&self) -> u8 {
        match self {
            Tag::Byte(_) => TAG_BYTE,
            Tag::Short(_) => TAG_SHORT,
            Tag::Int(_) => TAG_INT,
            Tag::Long(_) => TAG_LONG,
            Tag::Float(_) => TAG_FLOAT,
            Tag::Double(_) => TAG_DOUBLE,
            Tag::ByteArray(_) => TAG_BYTE_ARRAY,
            Tag::String(_) => TAG_STRING,
            Tag::List { .. } => TAG_LIST,
            Tag::Compound(_) => TAG_COMPOUND,
            Tag::IntArray(_) => TAG_INT_ARRAY,
            Tag::LongArray(_) => TAG_LONG_ARRAY,
        }
    }

    pub fn as_compound(&self) -> Option<&BTreeMap<String, Tag>> {
        match self {
            Tag::Compound(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_compound_mut(&mut self) -> Option<&mut BTreeMap<String, Tag>> {
        match self {
            Tag::Compound(map) => Some(map),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.as_compound()?.get(key)
    }

    /// 按路径逐层取子标签，例如 `get_path(&["Data", "Version", "Name"])`。
    pub fn get_path(&self, path: &[&str]) -> Option<&Tag> {
        let mut current = self;
        for key in path {
            current = current.get(key)?;
        }
        Some(current)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Tag::Byte(v) => Some(*v as i64),
            Tag::Short(v) => Some(*v as i64),
            Tag::Int(v) => Some(*v as i64),
            Tag::Long(v) => Some(*v),
            _ => None,
        }
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get_i64(key).map(|v| v as i32)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_i64(key).map(|v| v != 0)
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        self.get_str(key).map(str::to_string)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NbtFile {
    pub root_name: String,
    pub root: Tag,
}

impl NbtFile {
    pub fn read_gzip(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path)
            .map_err(|e| format!("打开 NBT 文件失败 {}: {e}", path.display()))?;
        let decoder = GzDecoder::new(file);
        let mut bytes = Vec::new();
        decoder
            .take(MAX_NBT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("解压 NBT 失败 {}: {e}", path.display()))?;
        if bytes.len() as u64 > MAX_NBT_BYTES {
            return Err("NBT 解压后过大".to_string());
        }
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_NBT_BYTES {
            return Err("NBT 数据过大".to_string());
        }
        let mut reader = Reader {
            data: bytes,
            pos: 0,
            tags: 0,
        };
        let tag_type = reader.u8()?;
        if tag_type == TAG_END {
            return Err("NBT 根标签不能是 TAG_End".to_string());
        }
        let root_name = reader.string()?;
        let root = reader.tag(tag_type, 0)?;
        if reader.pos != bytes.len() {
            // 尾部多余字节通常意味着文件损坏；但不影响已解析内容，给出警告即可
            eprintln!(
                "NBT 文件尾部有 {} 字节多余数据，已忽略",
                bytes.len() - reader.pos
            );
        }
        Ok(Self { root_name, root })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        out.push(self.root.type_id());
        write_string(&mut out, &self.root_name)?;
        write_payload(&mut out, &self.root, 0)?;
        Ok(out)
    }

    pub fn write_gzip(&self, path: &Path) -> Result<(), String> {
        let bytes = self.to_bytes()?;
        let tmp = {
            let mut name = path.as_os_str().to_os_string();
            name.push(".part");
            std::path::PathBuf::from(name)
        };
        if let Some(parent) = tmp.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
        }
        let file = std::fs::File::create(&tmp)
            .map_err(|e| format!("创建 NBT 文件失败 {}: {e}", tmp.display()))?;
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder
            .write_all(&bytes)
            .map_err(|e| format!("写入 NBT 失败 {}: {e}", tmp.display()))?;
        encoder
            .finish()
            .map_err(|e| format!("压缩 NBT 失败 {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("替换 NBT 文件失败 {}: {e}", path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 读取
// ---------------------------------------------------------------------------

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    tags: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "NBT 长度溢出".to_string())?;
        if end > self.data.len() {
            return Err(format!(
                "NBT 数据截断（需要 {len} 字节，剩余 {}）",
                self.data.len().saturating_sub(self.pos)
            ));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn i8(&mut self) -> Result<i8, String> {
        Ok(self.take(1)?[0] as i8)
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Result<i16, String> {
        let b = self.take(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    fn i32(&mut self) -> Result<i32, String> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&mut self) -> Result<i64, String> {
        let b = self.take(8)?;
        Ok(i64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.i32()? as u32))
    }

    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.i64()? as u64))
    }

    fn string(&mut self) -> Result<String, String> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        Ok(decode_modified_utf8(bytes))
    }

    fn tag(&mut self, type_id: u8, depth: usize) -> Result<Tag, String> {
        self.tags += 1;
        if self.tags > MAX_TAGS {
            return Err("NBT 标签数量超过预算".to_string());
        }
        if depth > MAX_DEPTH {
            return Err("NBT 嵌套过深".to_string());
        }
        match type_id {
            TAG_BYTE => Ok(Tag::Byte(self.i8()?)),
            TAG_SHORT => Ok(Tag::Short(self.i16()?)),
            TAG_INT => Ok(Tag::Int(self.i32()?)),
            TAG_LONG => Ok(Tag::Long(self.i64()?)),
            TAG_FLOAT => Ok(Tag::Float(self.f32()?)),
            TAG_DOUBLE => Ok(Tag::Double(self.f64()?)),
            TAG_BYTE_ARRAY => {
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_LIST_LEN {
                    return Err(format!("ByteArray 长度非法: {len}"));
                }
                Ok(Tag::ByteArray(self.take(len as usize)?.to_vec()))
            }
            TAG_STRING => Ok(Tag::String(self.string()?)),
            TAG_LIST => {
                let element_type = self.u8()?;
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_LIST_LEN {
                    return Err(format!("List 长度非法: {len}"));
                }
                if len as usize > MAX_TAGS.saturating_sub(self.tags) {
                    return Err("NBT 标签数量超过预算".to_string());
                }
                let mut items = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    items.push(self.tag(element_type, depth + 1)?);
                }
                Ok(Tag::List {
                    element_type,
                    items,
                })
            }
            TAG_COMPOUND => {
                let mut map = BTreeMap::new();
                loop {
                    let child_type = self.u8()?;
                    if child_type == TAG_END {
                        break;
                    }
                    let name = self.string()?;
                    let value = self.tag(child_type, depth + 1)?;
                    map.insert(name, value);
                }
                Ok(Tag::Compound(map))
            }
            TAG_INT_ARRAY => {
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_LIST_LEN {
                    return Err(format!("IntArray 长度非法: {len}"));
                }
                let mut values = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    values.push(self.i32()?);
                }
                Ok(Tag::IntArray(values))
            }
            TAG_LONG_ARRAY => {
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_LIST_LEN {
                    return Err(format!("LongArray 长度非法: {len}"));
                }
                let mut values = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    values.push(self.i64()?);
                }
                Ok(Tag::LongArray(values))
            }
            TAG_END => Err("意外的 TAG_End".to_string()),
            other => Err(format!("未知 NBT 标签类型: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// 写入
// ---------------------------------------------------------------------------

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let bytes = encode_modified_utf8(value);
    if bytes.len() > u16::MAX as usize {
        return Err(format!("NBT 字符串过长（{} 字节）", bytes.len()));
    }
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(())
}

/// NBT 使用 Java 的 modified UTF-8：NUL 编码为 `C0 80`，补充平面字符
/// （emoji 等）按 UTF-16 代理对拆成两个 3 字节序列。普通 UTF-8 对中文等价，
/// 但对 emoji / NUL 不等价，必须显式处理。
fn encode_modified_utf8(value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len());
    for unit in value.encode_utf16() {
        match unit {
            0 => out.extend_from_slice(&[0xC0, 0x80]),
            0x0001..=0x007F => out.push(unit as u8),
            0x0080..=0x07FF => {
                out.push(0xC0 | ((unit >> 6) as u8));
                out.push(0x80 | ((unit & 0x3F) as u8));
            }
            _ => {
                out.push(0xE0 | ((unit >> 12) as u8));
                out.push(0x80 | (((unit >> 6) & 0x3F) as u8));
                out.push(0x80 | ((unit & 0x3F) as u8));
            }
        }
    }
    out
}

fn decode_modified_utf8(bytes: &[u8]) -> String {
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b & 0x80 == 0 {
            units.push(b as u16);
            i += 1;
        } else if b & 0xE0 == 0xC0 {
            if i + 1 >= bytes.len() {
                units.push(0xFFFD);
                break;
            }
            let unit = (((b & 0x1F) as u16) << 6) | ((bytes[i + 1] & 0x3F) as u16);
            units.push(unit);
            i += 2;
        } else if b & 0xF0 == 0xE0 {
            if i + 2 >= bytes.len() {
                units.push(0xFFFD);
                break;
            }
            let unit = (((b & 0x0F) as u16) << 12)
                | (((bytes[i + 1] & 0x3F) as u16) << 6)
                | ((bytes[i + 2] & 0x3F) as u16);
            units.push(unit);
            i += 3;
        } else {
            units.push(0xFFFD);
            i += 1;
        }
    }
    String::from_utf16_lossy(&units)
}

fn write_payload(out: &mut Vec<u8>, tag: &Tag, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("NBT 嵌套过深".to_string());
    }
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::ByteArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            out.extend_from_slice(values);
        }
        Tag::String(value) => write_string(out, value)?,
        Tag::List {
            element_type,
            items,
        } => {
            out.push(*element_type);
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for item in items {
                if item.type_id() != *element_type && !items.is_empty() {
                    return Err(format!(
                        "List 元素类型不一致: 声明 {element_type}, 实际 {}",
                        item.type_id()
                    ));
                }
                write_payload(out, item, depth + 1)?;
            }
        }
        Tag::Compound(map) => {
            for (name, value) in map {
                out.push(value.type_id());
                write_string(out, name)?;
                write_payload(out, value, depth + 1)?;
            }
            out.push(TAG_END);
        }
        Tag::IntArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        Tag::LongArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_decompressed_nbt() {
        let path = std::env::temp_dir().join(format!("large-nbt-{}.dat", uuid::Uuid::new_v4()));
        let file = std::fs::File::create(&path).unwrap();
        let mut gzip = GzEncoder::new(file, Compression::fast());
        gzip.write_all(&vec![0; 16 * 1024 * 1024 + 1]).unwrap();
        gzip.finish().unwrap();
        let error = NbtFile::read_gzip(&path).unwrap_err();
        assert!(error.contains("过大"), "{error}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_list_allocation_exceeding_tag_budget() {
        let bytes = [TAG_LIST, 0, 0, TAG_BYTE, 0, 0x7a, 0x12, 0x00];
        let error = NbtFile::from_bytes(&bytes).unwrap_err();
        assert!(error.contains("预算"), "{error}");
    }

    #[test]
    fn rejects_oversized_direct_nbt_buffer() {
        let bytes = vec![0; MAX_NBT_BYTES as usize + 1];
        assert!(NbtFile::from_bytes(&bytes).unwrap_err().contains("过大"));
    }

    fn sample() -> NbtFile {
        let mut data = BTreeMap::new();
        data.insert("LevelName".to_string(), Tag::String("测试世界".to_string()));
        data.insert("GameType".to_string(), Tag::Int(1));
        data.insert("LastPlayed".to_string(), Tag::Long(1_793_000_000_000));
        data.insert(
            "difficulty_settings".to_string(),
            Tag::Compound(BTreeMap::from([
                ("difficulty".to_string(), Tag::String("easy".to_string())),
                ("hardcore".to_string(), Tag::Byte(0)),
            ])),
        );
        data.insert(
            "version_history".to_string(),
            Tag::List {
                element_type: TAG_INT,
                items: vec![Tag::Int(5023)],
            },
        );
        data.insert(
            "empty".to_string(),
            Tag::List {
                element_type: TAG_STRING,
                items: Vec::new(),
            },
        );
        NbtFile {
            root_name: String::new(),
            root: Tag::Compound(BTreeMap::from([("Data".to_string(), Tag::Compound(data))])),
        }
    }

    #[test]
    fn roundtrip_preserves_types() {
        let file = sample();
        let bytes = file.to_bytes().unwrap();
        let parsed = NbtFile::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(
            parsed.root.get_path(&["Data", "LevelName"]),
            Some(&Tag::String("测试世界".to_string()))
        );
        let empty = parsed.root.get_path(&["Data", "empty"]).unwrap();
        assert!(matches!(
            empty,
            Tag::List {
                element_type: TAG_STRING,
                items
            } if items.is_empty()
        ));
    }

    #[test]
    fn truncated_input_errors_instead_of_panicking() {
        let bytes = sample().to_bytes().unwrap();
        for cut in 1..bytes.len().min(30) {
            assert!(NbtFile::from_bytes(&bytes[..cut]).is_err());
        }
    }

    #[test]
    fn modified_utf8_roundtrips_chinese_emoji_and_nul() {
        for text in ["中文世界", "emoji 😀", "nul\0inside", "a😀中\0z"] {
            let bytes = encode_modified_utf8(text);
            assert_eq!(decode_modified_utf8(&bytes), text, "text={text:?}");
        }
        // NUL 必须是 C0 80，而不是 00
        assert_eq!(encode_modified_utf8("\0"), vec![0xC0, 0x80]);
    }

    #[test]
    fn parses_real_26_3_level_dat() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat");
        let file = NbtFile::read_gzip(&path).expect("parse fixture");
        assert_eq!(
            file.root.get_path(&["Data", "LevelName"]),
            Some(&Tag::String("world".to_string()))
        );
        assert_eq!(
            file.root.get_path(&["Data", "DataVersion"]),
            Some(&Tag::Int(5023))
        );
        assert_eq!(
            file.root.get_path(&["Data", "Version", "Name"]),
            Some(&Tag::String("26.3".to_string()))
        );
    }
}
