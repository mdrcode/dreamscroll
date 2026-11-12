//! Whether a candidate is the right *kind* of thing.
//!
//! A name can match an item that is a different sort of entity entirely: `Dogma` the 1999 film
//! against the concept, `Micah` the prophet against the given name, `Irene` a character against a
//! hurricane. Nothing lexical separates those; `P31` does.
//!
//! The rule is deliberately one-sided. A candidate is rejected only when one of its classes is
//! **recognised** and belongs to a family other than the one its entity type expects. An unknown
//! class never rejects, so the filter cannot lose a candidate because our table is incomplete — it
//! can only decline one we can show is a different kind of thing. That is the opposite of a type
//! *gate*, and it is why this does not need subclass traversal: `animated film` is unrecognised and
//! passes, which is a miss for us and never a wrong identity.
//!
//! Every QID below was checked against Wikidata on 2026-11-07.

/// The kind of thing a class says an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    /// A human being.
    Person,
    /// A created work: film, book, album, series, painting.
    Work,
    /// A fictional character.
    Fictional,
    /// An executable thing.
    Software,
    /// A made object that is not a work of authorship: a weapon, a vehicle, a tool.
    Artifact,
    /// Somewhere people live or a territory.
    Place,
    /// A collective, business or ensemble.
    Organization,
    /// Something that happens.
    Event,
    /// An abstraction rather than a thing.
    Concept,
    /// A word used to name people.
    Name,
    /// A natural phenomenon or a taxon.
    Nature,
}

impl Family {
    fn of(class: &str) -> Option<Self> {
        Some(match class {
            "Q5" => Self::Person,

            "Q11424" => Self::Work,    // film
            "Q571" => Self::Work,      // book
            "Q7725634" => Self::Work,  // literary work
            "Q47461344" => Self::Work, // written work
            "Q482994" => Self::Work,   // album
            "Q169930" => Self::Work,   // extended play
            "Q5398426" => Self::Work,  // television series
            "Q3305213" => Self::Work,  // painting
            "Q838948" => Self::Work,   // work of art
            "Q386724" => Self::Work,   // work
            "Q13442814" => Self::Work, // scholarly article

            "Q95074" => Self::Fictional,    // fictional character
            "Q7397" => Self::Software,      // software
            "Q124078422" => Self::Artifact, // weapon type

            "Q515" => Self::Place,      // city
            "Q6256" => Self::Place,     // country
            "Q486972" => Self::Place,   // human settlement
            "Q12488913" => Self::Place, // kampung of Papua

            "Q43229" => Self::Organization,    // organization
            "Q4830453" => Self::Organization,  // business
            "Q215380" => Self::Organization,   // musical group
            "Q16334295" => Self::Organization, // group of humans

            "Q1656682" => Self::Event, // planned event
            "Q178561" => Self::Event,  // battle
            "Q3002150" => Self::Event, // aviation accident

            "Q151885" => Self::Concept,   // concept
            "Q1914636" => Self::Concept,  // activity
            "Q3535173" => Self::Concept,  // organizational life cycle
            "Q11862829" => Self::Concept, // academic discipline
            "Q4671286" => Self::Concept,  // academic major
            "Q19610271" => Self::Concept, // literature term

            "Q202444" => Self::Name,   // given name
            "Q12308941" => Self::Name, // male given name
            "Q3409032" => Self::Name,  // unisex given name
            "Q101352" => Self::Name,   // family name
            "Q82799" => Self::Name,    // name

            "Q8092" => Self::Nature,  // tropical cyclone
            "Q16521" => Self::Nature, // taxon

            _ => return None,
        })
    }
}

/// Whether `classes` rule the candidate out for this entity type.
///
/// True only when at least one class is recognised and **every** recognised one is a kind of thing
/// this entity cannot be. Unknown classes and unmapped entity types never reject, so an incomplete
/// table can cost a rejection but never a correct identity.
///
/// The mapped types are the ones whose meaning we can defend. `publication`, `place`, `organization`
/// and `event` are deliberately unmapped: measured 2026-11-07, mapping them rejected correct identities
/// — `Rotten Tomatoes` is a business (`Q4830453`), the `Locarno Film Festival` an organization
/// (`Q43229`), the `Twenty-six Martyrs of Japan` a group of humans (`Q16334295`) — because a capture's
/// `entity_type` is a guess and Wikidata's classes are precise about a different axis.
pub(super) fn incompatible(entity_type: &str, classes: &[String]) -> bool {
    let found: std::collections::BTreeSet<Family> = classes
        .iter()
        .filter_map(|class| Family::of(class))
        .collect();
    if found.is_empty() {
        return false;
    }
    // Every arm below names a family this entity cannot be; the candidate is rejected only when
    // *every* recognised class falls in that set.
    let every_class_is = |families: fn(Family) -> bool| found.iter().copied().all(families);
    match entity_type {
        // A person is a human. A surname, a work, a concept or an organization is another kind of
        // thing — this is what caught `Kabugo` (a family name) and `Micah` (a given name).
        "real_person" | "religious_figure" => !found.contains(&Family::Person),
        // A created work is not a person, a place, a concept, a word, a creature or a program.
        // Organization and event are left out on purpose — see above.
        "movie" | "book" | "art_work" | "music" | "television_series" | "television_show" => {
            every_class_is(|family| {
                matches!(
                    family,
                    Family::Person
                        | Family::Place
                        | Family::Concept
                        | Family::Name
                        | Family::Nature
                        | Family::Software
                        | Family::Artifact
                )
            })
        }
        // A character is not a phenomenon, and not the work it appears in: `Irene` the character
        // against the hurricane, `Billy Costigan` against *The Departed*.
        "fictional_character" => every_class_is(|family| {
            matches!(
                family,
                Family::Nature
                    | Family::Work
                    | Family::Place
                    | Family::Organization
                    | Family::Event
                    | Family::Software
                    | Family::Artifact
                    | Family::Concept
                    | Family::Name
            )
        }),
        // A piece of software claiming a bird, a company or an album is another thing entirely.
        "software" => !found.contains(&Family::Software),
        // A product is not an event and not a person: `Boeing B-17 Flying Fortress` claimed
        // *2019 Boeing B-17 Flying Fortress crash* because "heavy bomber" appears in a crash
        // report. Nothing else is mapped here — a product's own classes are broad and unmapped,
        // and the rule only has to catch a kind of thing a product cannot be.
        "product" => every_class_is(|family| matches!(family, Family::Event | Family::Person)),
        // A brand is not a word. `Westinghouse` claimed a family-name page (`Q110089588`, P31
        // `Q101352`) — the only thing that item asserts is that the word is a surname.
        "brand" => every_class_is(|family| family == Family::Name),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn a_film_is_not_a_concept() {
        assert!(incompatible("movie", &classes(&["Q151885"])));
    }

    #[test]
    fn a_film_is_a_film() {
        assert!(!incompatible("movie", &classes(&["Q11424"])));
    }

    /// The point of the direction: our table being incomplete must never lose a candidate.
    #[test]
    fn an_unknown_class_never_rejects() {
        assert!(!incompatible("movie", &classes(&["Q99999999"])));
        assert!(!incompatible("movie", &classes(&[])));
    }

    #[test]
    fn one_agreeing_class_is_enough() {
        assert!(!incompatible("movie", &classes(&["Q99999999", "Q11424"])));
    }

    #[test]
    fn the_cases_that_motivated_it() {
        // Micah the prophet against the given name.
        assert!(incompatible("religious_figure", &classes(&["Q202444"])));
        // Irene the character against the hurricane.
        assert!(incompatible("fictional_character", &classes(&["Q8092"])));
        // Freedom of Speech the painting against the concept.
        assert!(incompatible("art_work", &classes(&["Q151885"])));
        // The Joy of Cooking against its publisher.
        assert!(!incompatible("book", &classes(&["Q571"])));
    }

    /// The four class gaps the second sample named, one test each.
    #[test]
    fn the_classes_the_sample_named() {
        // Revolver (the Beatles album) against `Q170382`, a handgun: class `weapon type`.
        assert!(incompatible("music", &classes(&["Q124078422"])));
        // Liquidation (a 2007 series) against the corporate concept.
        assert!(incompatible("television_show", &classes(&["Q3535173"])));
        // MUME (a MUD) against a Papuan village.
        assert!(incompatible("software", &classes(&["Q12488913"])));
        // Poetics (Aristotle) against the discipline.
        assert!(incompatible("book", &classes(&["Q11862829"])));
    }

    /// The class was missing from the table, not from Wikidata: `Micah` is a *male* given name
    /// (`Q12308941`), and the table only knew `given name` (`Q202444`).
    #[test]
    fn a_male_given_name_is_still_a_name() {
        assert!(incompatible("religious_figure", &classes(&["Q12308941"])));
        assert!(incompatible("real_person", &classes(&["Q3409032"])));
    }

    /// `Westinghouse`: a brand entity against a family-name item.
    #[test]
    fn a_brand_is_not_a_surname_page() {
        assert!(incompatible("brand", &classes(&["Q101352"])));
        // …but a brand whose class we do not recognise is left alone.
        assert!(incompatible("product", &classes(&["Q3002150"])));
        assert!(!incompatible("product", &classes(&["Q15056993"])));
        assert!(!incompatible("brand", &classes(&["Q99999999"])));
    }

    /// A type we have no opinion about is never filtered.
    #[test]
    fn an_unmapped_type_never_rejects() {
        assert!(!incompatible("brand", &classes(&["Q151885"])));
        assert!(!incompatible("unknown", &classes(&["Q151885"])));
    }

    /// Measured on the first rebuild: these are correct identities the filter must not reject.
    #[test]
    fn a_publication_may_be_a_business() {
        // Rotten Tomatoes, `Q105584`, P31 `business`.
        assert!(!incompatible("publication", &classes(&["Q4830453"])));
    }

    #[test]
    fn an_event_may_be_an_organization_or_a_group_of_people() {
        // Locarno Film Festival, `Q502053`.
        assert!(!incompatible("event", &classes(&["Q43229"])));
        // Twenty-six Martyrs of Japan, `Q578963`.
        assert!(!incompatible("event", &classes(&["Q16334295"])));
    }

    #[test]
    fn a_work_that_also_asserts_an_organization_is_not_rejected() {
        // Every recognised class has to disagree before a candidate is refused.
        assert!(!incompatible("movie", &classes(&["Q11424", "Q43229"])));
    }
}
