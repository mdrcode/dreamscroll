use serde::{Deserialize, Serialize};

/// One line of an export stream.
///
/// `user_id` sits on the line rather than on the response document so a single
/// stream can carry several users while `T` stays an unmodified `*Info` type.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExportRow<T> {
    pub user_id: i32,
    pub row: T,
}
