//! The SPARQL this authority sends, and nothing else.
//!
//! Pure string building, so a query change is visible in a diff and testable without a network.

use crate::fetch::Request;

use super::LANGUAGE;
use super::RequestKind;

pub(super) fn for_labels(requests: &[Request<RequestKind>]) -> String {
    let values = requests
        .iter()
        .map(|request| format!("\"{}\"@{LANGUAGE}", escape(&request.query)))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        // `P31` is asked for on every label answer because `types::incompatible` is the gate that
        // keeps the right name on the wrong kind of thing out (`Dogma` the film against the
        // concept). `OPTIONAL` because an item with no class is still a label match, and a missing
        // class must not drop the candidate. Measured 2026-11-07: the join costs nothing (1.9 s
        // against 2.5 s at 25 labels, inside the noise), though the response is ~37% larger
        // because an item with several classes answers once per class — hence the merge in
        // `transport::sparql`.
        "SELECT ?query ?item ?itemLabel ?itemDescription ?class WHERE {{\n\
         VALUES ?query {{ {values} }}\n\
         ?item rdfs:label ?query .\n\
         OPTIONAL {{ ?item wdt:P31 ?class . }}\n\
         SERVICE wikibase:label {{ bd:serviceParam wikibase:language \"{LANGUAGE}\". }}\n\
         }}"
    )
}
pub(super) fn for_handles(requests: &[Request<RequestKind>], property: &str) -> String {
    let values = requests
        .iter()
        .map(|request| format!("\"{}\"", escape(&request.query)))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "SELECT ?query ?item WHERE {{\n\
         VALUES ?query {{ {values} }}\n\
         ?item wdt:{property} ?query .\n\
         }}"
    )
}

/// SPARQL string literals: backslash and double quote are the only escapes that matter here.
pub(super) fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::identity::wikidata::RequestKind;

    fn request(kind: RequestKind, query: &str) -> Request<RequestKind> {
        Request {
            provider: "wikidata",
            kind,
            shape: "test".to_string(),
            query: query.to_string(),
        }
    }

    /// SPARQL literals: a quote or a backslash must not end the value.
    #[test]
    fn a_literal_is_escaped() {
        assert_eq!(escape("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn a_label_query_asks_for_every_label_in_one_values_block() {
        let query = for_labels(&[request(RequestKind::Label, "Train Dreams")]);
        assert!(query.contains("\"Train Dreams\"@en"), "{query}");
        assert!(query.contains("rdfs:label"), "{query}");
    }

    #[test]
    fn a_handle_query_asks_about_the_property_that_holds_the_account() {
        let query = for_handles(&[request(RequestKind::Handle("P2002"), "tsoding")], "P2002");
        assert!(query.contains("wdt:P2002"), "{query}");
        assert!(query.contains("\"tsoding\""), "{query}");
    }
}
