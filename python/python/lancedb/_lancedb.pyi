from typing import Optional

import pyarrow as pa

class Connection(object):
    async def table_names(
        self, start_after: Optional[str], limit: Optional[int]
    ) -> list[str]: ...
    async def create_table(
        self, name: str, mode: str, data: pa.RecordBatchReader
    ) -> Table: ...
    async def create_empty_table(
        self, name: str, mode: str, schema: pa.Schema
    ) -> Table: ...

class Table(object):
    def name(self) -> str: ...
    def __repr__(self) -> str: ...
    async def schema(self) -> pa.Schema: ...

async def connect(
    uri: str,
    api_key: Optional[str],
    region: Optional[str],
    host_override: Optional[str],
    read_consistency_interval: Optional[float],
) -> Connection: ...
