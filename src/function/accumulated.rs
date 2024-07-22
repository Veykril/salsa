use crate::{accumulator, hash::FxHashSet, storage::DatabaseGen, DatabaseKeyIndex, Id};
use std::collections::VecDeque;

use super::{Configuration, IngredientImpl};

impl<C> IngredientImpl<C>
where
    C: Configuration,
{
    /// Helper used by `accumulate` functions. Computes the results accumulated by `database_key_index`
    /// and its inputs.
    pub fn accumulated_by<A>(&self, db: &C::DbView, key: Id) -> Vec<A>
    where
        A: accumulator::Accumulator,
    {
        let Some(accumulator) = <accumulator::IngredientImpl<A>>::from_db(db) else {
            return vec![];
        };
        let runtime = db.runtime();
        let mut output = vec![];

        // First ensure the result is up to date
        self.fetch(db, key);

        let db_key = self.database_key_index(key);
        let mut visited: FxHashSet<DatabaseKeyIndex> = std::iter::once(db_key).collect();
        let mut queue: VecDeque<DatabaseKeyIndex> = std::iter::once(db_key).collect();

        while let Some(k) = queue.pop_front() {
            accumulator.produced_by(runtime, k, &mut output);

            let origin = db.lookup_ingredient(k.ingredient_index).origin(k.key_index);
            let inputs = origin.iter().flat_map(|origin| origin.inputs());

            for input in inputs {
                if let Ok(input) = input.try_into() {
                    if visited.insert(input) {
                        queue.push_back(input);
                    }
                }
            }
        }

        output
    }
}
