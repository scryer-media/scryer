//! The stored form of one library graph: nodes, edges and clusters packed
//! into one zstd-compressed blob per run.
//!
//! Every read of a graph loads it whole, so the graph is stored whole. The
//! blob holds one sorted table of every distinct string the graph names, and
//! nodes, edges and clusters refer to those strings by index:
//!
//! ```text
//! u8      format version
//! varint  string count, then per string: varint byte length, UTF-8 bytes
//! varint  node count, then per node: subject, target, zigzag cluster id,
//!         title id count, title ids
//! varint  kind list count, then per list: length, kinds (in stored order)
//! varint  source count, then per source (an edge's from key, ascending):
//!         delta from the previous source, degree, then per edge: delta
//!         from the previous target (the first target absolute), kind list
//! u64 x E edge weights in edge order, split into eight byte planes
//! varint  cluster count, then per cluster: zigzag id, label key + 1 (0 for
//!         none), label, medoid, zigzag size
//! ```
//!
//! Because the string table is sorted, ordering by string index is ordering
//! by key, so the edge list doubles as an adjacency list (CSR) ordered by
//! `(from_key, to_key)`. Decoding returns nodes by subject key, edges by
//! `(from_key, to_key)` and clusters by id, each deduplicated on its key with
//! the first occurrence kept.

use std::collections::{BTreeSet, HashMap, HashSet};

use scryer_application::{
    AppError, AppResult, LibraryGraphClusterRecord, LibraryGraphEdgeRecord, LibraryGraphNodeRecord,
};

const PAYLOAD_FORMAT_V1: u8 = 1;

/// Compression level of the stored blob. Measured on a 10,000-subject graph,
/// higher levels saved under 1% and took several times longer: what is left
/// after level 3 is mostly edge weights, which do not compress.
const PAYLOAD_COMPRESSION_LEVEL: i32 = 3;

/// Largest decoded payload a read accepts, well above a full-size graph.
const PAYLOAD_MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;

/// A library graph's children, in stored order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct LibraryGraphPayload {
    pub(super) nodes: Vec<LibraryGraphNodeRecord>,
    pub(super) edges: Vec<LibraryGraphEdgeRecord>,
    pub(super) clusters: Vec<LibraryGraphClusterRecord>,
}

fn corrupt(detail: &str) -> AppError {
    AppError::Repository(format!("stored library graph is unreadable: {detail}"))
}

/// Encodes a graph's children, compressed.
pub(super) fn encode_library_graph_payload(
    nodes: &[LibraryGraphNodeRecord],
    edges: &[LibraryGraphEdgeRecord],
    clusters: &[LibraryGraphClusterRecord],
) -> AppResult<Vec<u8>> {
    let raw = encode_raw(nodes, edges, clusters)?;
    scryer_infrastructure_sql::compressed_blob::compress_blob(&raw, PAYLOAD_COMPRESSION_LEVEL)
}

/// Decodes what [`encode_library_graph_payload`] stored.
pub(super) fn decode_library_graph_payload(encoded: &[u8]) -> AppResult<LibraryGraphPayload> {
    let raw = scryer_infrastructure_sql::compressed_blob::decompress_blob(
        encoded,
        PAYLOAD_MAX_DECODED_BYTES,
    )?;
    decode_raw(&raw)
}

fn first_by_key<'a, T, K: Eq + std::hash::Hash>(
    items: &'a [T],
    key: impl Fn(&'a T) -> K,
) -> Vec<&'a T> {
    let mut seen = HashSet::new();
    items.iter().filter(|item| seen.insert(key(item))).collect()
}

fn encode_raw(
    nodes: &[LibraryGraphNodeRecord],
    edges: &[LibraryGraphEdgeRecord],
    clusters: &[LibraryGraphClusterRecord],
) -> AppResult<Vec<u8>> {
    let mut nodes = first_by_key(nodes, |node| node.subject_key.as_str());
    nodes.sort_by(|left, right| left.subject_key.cmp(&right.subject_key));
    let mut edges = first_by_key(edges, |edge| (edge.from_key.as_str(), edge.to_key.as_str()));
    edges.sort_by(|left, right| {
        (left.from_key.as_str(), left.to_key.as_str())
            .cmp(&(right.from_key.as_str(), right.to_key.as_str()))
    });
    let mut clusters = first_by_key(clusters, |cluster| cluster.cluster_id);
    clusters.sort_by_key(|cluster| cluster.cluster_id);

    let mut strings = BTreeSet::<&str>::new();
    for node in &nodes {
        strings.insert(&node.subject_key);
        strings.insert(&node.target_key);
        strings.extend(node.title_ids.iter().map(String::as_str));
    }
    for edge in &edges {
        strings.insert(&edge.from_key);
        strings.insert(&edge.to_key);
        strings.extend(edge.kinds.iter().map(String::as_str));
    }
    for cluster in &clusters {
        strings.extend(cluster.label_key.as_deref());
        strings.insert(&cluster.label);
        strings.insert(&cluster.medoid);
    }
    let index = strings
        .iter()
        .enumerate()
        .map(|(position, value)| (*value, position as u64))
        .collect::<HashMap<_, _>>();

    let mut out = Writer::default();
    out.byte(PAYLOAD_FORMAT_V1);
    out.len(strings.len());
    for value in &strings {
        out.len(value.len());
        out.bytes(value.as_bytes());
    }

    out.len(nodes.len());
    for node in &nodes {
        out.varint(index[node.subject_key.as_str()]);
        out.varint(index[node.target_key.as_str()]);
        out.zigzag(node.cluster_id);
        out.len(node.title_ids.len());
        for title_id in &node.title_ids {
            out.varint(index[title_id.as_str()]);
        }
    }

    let mut kind_lists = Vec::<&[String]>::new();
    let mut kind_list_index = HashMap::<&[String], u64>::new();
    let edge_kind_lists = edges
        .iter()
        .map(|edge| {
            *kind_list_index
                .entry(edge.kinds.as_slice())
                .or_insert_with(|| {
                    kind_lists.push(edge.kinds.as_slice());
                    (kind_lists.len() - 1) as u64
                })
        })
        .collect::<Vec<_>>();
    out.len(kind_lists.len());
    for kinds in &kind_lists {
        out.len(kinds.len());
        for kind in *kinds {
            out.varint(index[kind.as_str()]);
        }
    }

    let edge_ends = edges
        .iter()
        .map(|edge| (index[edge.from_key.as_str()], index[edge.to_key.as_str()]))
        .collect::<Vec<_>>();
    let mut sources = Vec::<(u64, usize)>::new();
    for (from, _) in &edge_ends {
        match sources.last_mut() {
            Some((source, degree)) if source == from => *degree += 1,
            _ => sources.push((*from, 1)),
        }
    }
    out.len(sources.len());
    let mut position = 0;
    let mut previous_source = 0;
    for (source, degree) in sources {
        out.varint(source - previous_source);
        previous_source = source;
        out.len(degree);
        let mut previous_target = 0;
        for offset in 0..degree {
            let target = edge_ends[position + offset].1;
            out.varint(target - previous_target);
            previous_target = target;
            out.varint(edge_kind_lists[position + offset]);
        }
        position += degree;
    }
    for plane in 0..8 {
        for edge in &edges {
            out.byte(edge.weight.to_bits().to_le_bytes()[plane]);
        }
    }

    out.len(clusters.len());
    for cluster in &clusters {
        out.zigzag(cluster.cluster_id);
        out.varint(
            cluster
                .label_key
                .as_deref()
                .map_or(0, |label_key| index[label_key] + 1),
        );
        out.varint(index[cluster.label.as_str()]);
        out.varint(index[cluster.medoid.as_str()]);
        out.zigzag(cluster.size);
    }
    Ok(out.0)
}

fn decode_raw(raw: &[u8]) -> AppResult<LibraryGraphPayload> {
    let mut input = Reader { raw, position: 0 };
    let format = input.byte()?;
    if format != PAYLOAD_FORMAT_V1 {
        return Err(corrupt(&format!("unknown format {format}")));
    }
    let string_count = input.len()?;
    let mut strings = Vec::new();
    for _ in 0..string_count {
        let length = input.len()?;
        let bytes = input.bytes(length)?;
        strings.push(
            std::str::from_utf8(bytes)
                .map_err(|_| corrupt("a string is not UTF-8"))?
                .to_string(),
        );
    }
    let string = |index: u64| -> AppResult<String> {
        usize::try_from(index)
            .ok()
            .and_then(|index| strings.get(index))
            .cloned()
            .ok_or_else(|| corrupt("a string index is out of range"))
    };

    let node_count = input.len()?;
    let mut nodes = Vec::new();
    for _ in 0..node_count {
        let subject_key = string(input.varint()?)?;
        let target_key = string(input.varint()?)?;
        let cluster_id = input.zigzag()?;
        let title_count = input.len()?;
        let mut title_ids = Vec::new();
        for _ in 0..title_count {
            title_ids.push(string(input.varint()?)?);
        }
        nodes.push(LibraryGraphNodeRecord {
            subject_key,
            target_key,
            cluster_id,
            title_ids,
        });
    }

    let kind_list_count = input.len()?;
    let mut kind_lists = Vec::new();
    for _ in 0..kind_list_count {
        let length = input.len()?;
        let mut kinds = Vec::new();
        for _ in 0..length {
            kinds.push(string(input.varint()?)?);
        }
        kind_lists.push(kinds);
    }

    let source_count = input.len()?;
    let mut edges = Vec::new();
    let mut source = 0u64;
    for _ in 0..source_count {
        source = source
            .checked_add(input.varint()?)
            .ok_or_else(|| corrupt("a source index overflows"))?;
        let from_key = string(source)?;
        let degree = input.len()?;
        let mut target = 0u64;
        for _ in 0..degree {
            target = target
                .checked_add(input.varint()?)
                .ok_or_else(|| corrupt("a target index overflows"))?;
            let kinds = usize::try_from(input.varint()?)
                .ok()
                .and_then(|index| kind_lists.get(index))
                .cloned()
                .ok_or_else(|| corrupt("a kind list index is out of range"))?;
            edges.push(LibraryGraphEdgeRecord {
                from_key: from_key.clone(),
                to_key: string(target)?,
                weight: 0.0,
                kinds,
            });
        }
    }
    let edge_count = edges.len();
    let planes = input.bytes(edge_count * 8)?;
    for (position, edge) in edges.iter_mut().enumerate() {
        let mut bits = [0u8; 8];
        for (plane, byte) in bits.iter_mut().enumerate() {
            *byte = planes[plane * edge_count + position];
        }
        edge.weight = f64::from_bits(u64::from_le_bytes(bits));
    }

    let cluster_count = input.len()?;
    let mut clusters = Vec::new();
    for _ in 0..cluster_count {
        let cluster_id = input.zigzag()?;
        let label_key = match input.varint()? {
            0 => None,
            index => Some(string(index - 1)?),
        };
        clusters.push(LibraryGraphClusterRecord {
            cluster_id,
            label_key,
            label: string(input.varint()?)?,
            medoid: string(input.varint()?)?,
            size: input.zigzag()?,
        });
    }
    if input.position != raw.len() {
        return Err(corrupt("trailing bytes"));
    }
    Ok(LibraryGraphPayload {
        nodes,
        edges,
        clusters,
    })
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn byte(&mut self, value: u8) {
        self.0.push(value);
    }

    fn bytes(&mut self, value: &[u8]) {
        self.0.extend_from_slice(value);
    }

    fn varint(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.0.push((value as u8) | 0x80);
            value >>= 7;
        }
        self.0.push(value as u8);
    }

    fn len(&mut self, value: usize) {
        self.varint(value as u64);
    }

    fn zigzag(&mut self, value: i64) {
        self.varint(((value << 1) ^ (value >> 63)) as u64);
    }
}

struct Reader<'a> {
    raw: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> AppResult<u8> {
        let value = *self
            .raw
            .get(self.position)
            .ok_or_else(|| corrupt("truncated"))?;
        self.position += 1;
        Ok(value)
    }

    fn bytes(&mut self, length: usize) -> AppResult<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.raw.len())
            .ok_or_else(|| corrupt("truncated"))?;
        let value = &self.raw[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn varint(&mut self) -> AppResult<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(corrupt("a varint is too long"))
    }

    /// A count, bounded by the bytes left. Collections grow as elements are
    /// read rather than being presized, so a corrupt count cannot ask for a
    /// huge allocation up front either.
    fn len(&mut self) -> AppResult<usize> {
        usize::try_from(self.varint()?)
            .ok()
            .filter(|count| *count <= self.raw.len() - self.position)
            .ok_or_else(|| corrupt("a count is out of range"))
    }

    fn zigzag(&mut self) -> AppResult<i64> {
        let value = self.varint()?;
        Ok(((value >> 1) as i64) ^ -((value & 1) as i64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        subject: &str,
        target: &str,
        cluster_id: i64,
        titles: &[&str],
    ) -> LibraryGraphNodeRecord {
        LibraryGraphNodeRecord {
            subject_key: subject.to_string(),
            target_key: target.to_string(),
            cluster_id,
            title_ids: titles.iter().map(|title| title.to_string()).collect(),
        }
    }

    fn edge(from: &str, to: &str, weight: f64, kinds: &[&str]) -> LibraryGraphEdgeRecord {
        LibraryGraphEdgeRecord {
            from_key: from.to_string(),
            to_key: to.to_string(),
            weight,
            kinds: kinds.iter().map(|kind| kind.to_string()).collect(),
        }
    }

    #[test]
    fn library_graph_payload_round_trips_in_key_order_keeping_first_duplicates() {
        let nodes = vec![
            node("tmdb:movie:2", "tmdb:movie:2", 0, &["title-b", "title-b2"]),
            node("local:abc", "tmdb:movie:9", -1, &[]),
            node("tmdb:movie:1", "tmdb:movie:1", i64::MIN, &["title-a"]),
            node("tmdb:movie:2", "tmdb:movie:x", 5, &["title-dup"]),
        ];
        let edges = vec![
            edge(
                "tmdb:movie:2",
                "tmdb:movie:9",
                0.75,
                &["relation:list", "semantic"],
            ),
            edge("tmdb:movie:1", "tmdb:movie:2", 0.9, &["semantic"]),
            edge("tmdb:movie:1", "tmdb:movie:9", f64::MIN_POSITIVE, &[]),
            edge("tmdb:movie:1", "tmdb:movie:2", 0.1, &["duplicate"]),
            // A key no node names still round-trips.
            edge(
                "tmdb:movie:0",
                "tmdb:movie:1",
                -0.0,
                &["semantic", "relation:list"],
            ),
        ];
        let clusters = vec![
            LibraryGraphClusterRecord {
                cluster_id: 3,
                label_key: None,
                label: "Synthetic Title".to_string(),
                medoid: "tmdb:movie:2".to_string(),
                size: 1,
            },
            LibraryGraphClusterRecord {
                cluster_id: -2,
                label_key: Some("canonical:genre:heist".to_string()),
                label: String::new(),
                medoid: "tmdb:movie:1".to_string(),
                size: i64::MAX,
            },
            LibraryGraphClusterRecord {
                cluster_id: 3,
                label_key: None,
                label: "Duplicate".to_string(),
                medoid: "tmdb:movie:1".to_string(),
                size: 9,
            },
        ];
        let encoded = encode_library_graph_payload(&nodes, &edges, &clusters).expect("encode");
        let decoded = decode_library_graph_payload(&encoded).expect("decode");
        assert_eq!(
            decoded.nodes,
            vec![nodes[1].clone(), nodes[2].clone(), nodes[0].clone()]
        );
        assert_eq!(
            decoded.edges,
            vec![
                edges[4].clone(),
                edges[1].clone(),
                edges[2].clone(),
                edges[0].clone()
            ]
        );
        assert_eq!(decoded.edges[0].weight.to_bits(), (-0.0f64).to_bits());
        assert_eq!(
            decoded.clusters,
            vec![clusters[1].clone(), clusters[0].clone()]
        );
    }

    #[test]
    fn an_empty_library_graph_payload_round_trips() {
        let encoded = encode_library_graph_payload(&[], &[], &[]).expect("encode");
        assert_eq!(
            decode_library_graph_payload(&encoded).expect("decode"),
            LibraryGraphPayload::default()
        );
    }

    #[test]
    fn a_damaged_library_graph_payload_is_refused_rather_than_misread() {
        let nodes = vec![node("tmdb:movie:1", "tmdb:movie:1", 0, &["title-a"])];
        let edges = vec![edge("tmdb:movie:1", "tmdb:movie:2", 0.5, &["semantic"])];
        let raw = encode_raw(&nodes, &edges, &[]).expect("encode");
        for cut in 0..raw.len() {
            assert!(decode_raw(&raw[..cut]).is_err(), "truncated at {cut}");
        }
        let mut trailing = raw.clone();
        trailing.push(0);
        assert!(decode_raw(&trailing).is_err());
        let mut unknown_format = raw;
        unknown_format[0] = 0xff;
        assert!(decode_raw(&unknown_format).is_err());
        assert!(decode_library_graph_payload(b"not a zstd frame").is_err());
    }
}
