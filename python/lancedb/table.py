#  Copyright 2023 LanceDB Developers
#
#  Licensed under the Apache License, Version 2.0 (the "License");
#  you may not use this file except in compliance with the License.
#  You may obtain a copy of the License at
#      http://www.apache.org/licenses/LICENSE-2.0
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.

from __future__ import annotations

import os
from functools import cached_property

import lance
import numpy as np
import pandas as pd
from lance import LanceDataset
import pyarrow as pa
from lance.vector import vec_to_table

from .query import LanceQueryBuilder
from .common import DATA, VECTOR_COLUMN_NAME, VEC


def _sanitize_data(data, schema):
    if isinstance(data, list):
        data = pa.Table.from_pylist(data)
        data = _sanitize_schema(data, schema=schema)
    if isinstance(data, dict):
        data = vec_to_table(data)
    if isinstance(data, pd.DataFrame):
        data = pa.Table.from_pandas(data)
        data = _sanitize_schema(data, schema=schema)
    if not isinstance(data, pa.Table):
        raise TypeError(f"Unsupported data type: {type(data)}")
    return data


class LanceTable:
    """
    A table in a LanceDB database.
    """

    def __init__(self, connection: "lancedb.db.LanceDBConnection", name: str):
        self._conn = connection
        self.name = name

    @property
    def schema(self) -> pa.Schema:
        """Return the schema of the table."""
        return self._dataset.schema

    @property
    def _dataset_uri(self) -> str:
        return os.path.join(self._conn.uri, f"{self.name}.lance")

    @cached_property
    def _dataset(self) -> LanceDataset:
        return lance.dataset(self._dataset_uri)

    def to_lance(self) -> LanceDataset:
        """Return the LanceDataset backing this table."""
        return self._dataset

    def add(self, data: DATA, mode: str = "append") -> int:
        """Add data to the table.

        Parameters
        ----------
        data: list-of-dict, dict, pd.DataFrame
            The data to insert into the table.
        mode: str
            The mode to use when writing the data. Valid values are
            "append" and "overwrite".

        Returns
        -------
        The number of vectors added to the table.
        """
        data = _sanitize_data(data, self.schema)
        ds = lance.write_dataset(data, self._dataset_uri, mode=mode)
        return ds.count_rows()

    def search(self, query: VEC) -> LanceQueryBuilder:
        """Create a search query to find the nearest neighbors
        of the given query vector.

        Parameters
        ----------
        query: list, np.ndarray
            The query vector.

        Returns
        -------
        A LanceQueryBuilder object representing the query.
        """
        if isinstance(query, list):
            query = np.array(query)
        if isinstance(query, np.ndarray):
            query = query.astype(np.float32)
        else:
            raise TypeError(f"Unsupported query type: {type(query)}")
        return LanceQueryBuilder(self, query)

    @classmethod
    def create(cls, db, name, data, schema):
        tbl = LanceTable(db, name)
        data = _sanitize_data(data, schema)
        lance.write_dataset(data, tbl._dataset_uri, mode="create")
        return tbl


def _sanitize_schema(data: pa.Table, schema: pa.Schema = None) -> pa.Table:
    """Ensure that the table has the expected schema.

    Parameters
    ----------
    data: pa.Table
        The table to sanitize.
    schema: pa.Schema; optional
        The expected schema. If not provided, this just converts the
        vector column to fixed_size_list(float32) if necessary.
    """
    if schema is not None:
        if data.schema == schema:
            return data
        # cast the columns to the expected types
        data = data.combine_chunks()
        return pa.Table.from_arrays([
            data[name].cast(schema.field(name).type)
            for name in schema.names
        ], schema=schema)
    # just check the vector column
    return _sanitize_vector_column(data, vector_column_name=VECTOR_COLUMN_NAME)


def _sanitize_vector_column(data: pa.Table, vector_column_name: str) -> pa.Table:
    """
    Ensure that the vector column exists and has type fixed_size_list(float32)

    Parameters
    ----------
    data: pa.Table
        The table to sanitize.
    vector_column_name: str
        The name of the vector column.
    """
    i = data.column_names.index(vector_column_name)
    if i < 0:
        raise ValueError(f"Missing vector column: {vector_column_name}")
    vec_arr = data[vector_column_name].combine_chunks()
    if pa.types.is_fixed_size_list(vec_arr.type):
        return data
    if not pa.types.is_list(vec_arr.type):
        raise TypeError(f"Unsupported vector column type: {vec_arr.type}")
    values = vec_arr.values
    if not pa.types.is_float32(values.type):
        values = values.cast(pa.float32())
    list_size = len(values) / len(data)
    vec_arr = pa.FixedSizeListArray.from_arrays(values, list_size)
    return data.set_column(i, vector_column_name, vec_arr)
