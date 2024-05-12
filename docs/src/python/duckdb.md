# DuckDB

In Python, LanceDB tables can also be queried with [DuckDB](https://duckdb.org/), an in-process SQL OLAP database. This means you can write complex SQL queries to analyze your data in LanceDB.

This integration is done via [Apache Arrow](https://duckdb.org/docs/guides/python/sql_on_arrow), which provides zero-copy data sharing between LanceDB and DuckDB. DuckDB is capable of passing down column selections and basic filters to LanceDB, reducing the amount of data that needs to be scanned to perform your query. Finally, the integration allows streaming data from LanceDB tables, allowing you to aggregate tables that won't fit into memory. All of this uses the same mechanism described in DuckDB's blog post *[DuckDB quacks Arrow](https://duckdb.org/2021/12/03/duck-arrow.html)*.


We can demonstrate this by first installing `duckdb` and `lancedb`.

```shell
pip install duckdb lancedb
```

We will re-use the dataset [created previously](./pandas_and_pyarrow.md):

```python
import lancedb

db = lancedb.connect("data/sample-lancedb")
data = [
    {"vector": [3.1, 4.1], "item": "foo", "price": 10.0},
    {"vector": [5.9, 26.5], "item": "bar", "price": 20.0}
]
table = db.create_table("pd_table", data=data)
```

The `to_lance` method converts the LanceDB table to a `LanceDataset`, which is accessible to DuckDB through the Arrow compatibility layer.
To query the resulting Lance dataset in DuckDB, all you need to do is reference the dataset by the same name in your SQL query.

```python
import duckdb

arrow_table = table.to_lance()

duckdb.query("SELECT * FROM arrow_table")
```

```
┌─────────────┬─────────┬────────┐
│   vector    │  item   │ price  │
│   float[]   │ varchar │ double │
├─────────────┼─────────┼────────┤
│ [3.1, 4.1]  │ foo     │   10.0 │
│ [5.9, 26.5] │ bar     │   20.0 │
└─────────────┴─────────┴────────┘
```

You can very easily run any other DuckDB SQL queries on your data.

```py
duckdb.query("SELECT mean(price) FROM arrow_table")
```

```
┌─────────────┐
│ mean(price) │
│   double    │
├─────────────┤
│        15.0 │
└─────────────┘
```