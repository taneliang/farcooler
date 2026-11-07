//! Exact typed conservation evidence, not an archive or an execution authority.
use std::collections::BTreeMap;
use rusqlite::{Connection, types::ValueRef};
use serde::{Deserialize, Serialize};
use super::Result;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cell {
    Null,
    Integer(i64),
    Real(u64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}
impl Cell {
    pub fn text(&self) -> Option<&str> {
        match self { Self::Text(v) => std::str::from_utf8(v).ok(), _ => None }
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        match self { Self::Blob(v) => Some(v), _ => None }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub declared_type: String,
    pub not_null: bool,
    pub default: Option<String>,
    pub primary_key_order: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    pub rowids: Option<BTreeMap<String, Vec<i64>>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub objects: BTreeMap<String, (String, String, Option<String>)>,
    pub tables: BTreeMap<String, Table>,
}
/// Quote only discovered SQLite identifiers, never interpret stored resource paths.
pub(super) fn ident(name: &str) -> String { format!("\"{}\"", name.replace('"', "\"\"")) }
impl Manifest {
    pub fn capture(conn: &Connection) -> Result<Self> {
        let mut objects = BTreeMap::new();
        let mut tables = BTreeMap::new();
        let mut stmt = conn.prepare("SELECT name,type,tbl_name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' OR name='sqlite_sequence' ORDER BY name")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))? {
            let (name, kind, owner, sql) = row?;
            if kind == "table" {
                let mut columns = Vec::new();
                let mut info = conn.prepare(&format!("PRAGMA table_info({})", ident(&name)))?;
                for col in info.query_map([], |r| Ok(Column { name:r.get(1)?, declared_type:r.get(2)?, not_null:r.get(3)?, default:r.get(4)?, primary_key_order:r.get(5)? }))? { columns.push(col?); }
                let has_rowid=conn.prepare(&format!("SELECT _rowid_ FROM {} LIMIT 0",ident(&name))).is_ok();
                let mut rowids=has_rowid.then(BTreeMap::<String,Vec<i64>>::new);
                let prefix=if has_rowid {"_rowid_,"} else {""};
                let mut select = conn.prepare(&format!("SELECT {prefix}* FROM {}", ident(&name)))?;
                let mut cursor = select.query([])?;
                let mut rows = Vec::new();
                while let Some(row) = cursor.next()? {
                    let mut values = Vec::new();
                    for index in 0..columns.len() {
                        values.push(match row.get_ref(index+usize::from(has_rowid))? {
                            ValueRef::Null => Cell::Null,
                            ValueRef::Integer(v) => Cell::Integer(v),
                            ValueRef::Real(v) => Cell::Real(v.to_bits()),
                            ValueRef::Text(v) => Cell::Text(v.to_vec()),
                            ValueRef::Blob(v) => Cell::Blob(v.to_vec()),
                        });
                    }
                    if let Some(ids)=rowids.as_mut() {ids.entry(identity(&columns,&values)).or_default().push(row.get(0)?);}
                    rows.push(values);
                }
                rows.sort_by_cached_key(|row| serde_json::to_vec(row).expect("closed cells"));
                if let Some(ids)=rowids.as_mut() {for values in ids.values_mut() {values.sort_unstable();}}
                tables.insert(name.clone(), Table { columns, rows, rowids });
            }
            objects.insert(name, (kind, owner, sql));
        }
        Ok(Self { objects, tables })
    }
    pub fn hash(&self) -> String { farcooler_workflow::digest(self) }
    /// No ignored fields. Only the two exact migration meta keys may differ.
    /// All existing DDL, keys, relationship columns, JSON bytes and allocators match.
    pub fn conserved_by(&self, after: &Self) -> bool {
        self.objects.iter().all(|(k,v)| after.objects.get(k) == Some(v)) &&
        self.tables.iter().all(|(name, before)| {
            let Some(current) = after.tables.get(name) else { return false };
            if name != "meta" { return before == current; }
            let without_stamps = |table: &Table| table.rows.iter().filter(|r|
                !matches!(r.first().and_then(Cell::text), Some("schema_version" | "compatible_down_to")))
                .cloned().collect::<Vec<_>>();
            let ids_without_stamps=|table:&Table| table.rowids.as_ref().map(|ids| without_stamps(table).iter().map(|row| {
                let key=identity(&table.columns,row);let values=ids.get(&key).cloned();(key,values)
            }).collect::<BTreeMap<_,_>>());
            before.columns == current.columns && without_stamps(before) == without_stamps(current) && ids_without_stamps(before)==ids_without_stamps(current)
        })
    }
}

pub(super) fn identity(columns:&[Column],row:&[Cell])->String {
    let keys:Vec<_>=columns.iter().enumerate().filter(|(_,c)|c.primary_key_order>0).map(|(i,c)|(c.primary_key_order,&row[i])).collect();
    if keys.is_empty() {farcooler_workflow::digest(&row)} else {farcooler_workflow::digest(&keys)}
}
