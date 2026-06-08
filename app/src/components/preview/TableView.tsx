import { useMemo } from 'react';
import Papa from 'papaparse';
import { useTranslation } from 'react-i18next';

const MAX_ROWS = 1000;

interface Props {
  content: string;
  /** '\t' for .tsv; '' lets papaparse auto-detect (the default for .csv). */
  delimiter?: string;
}

// Cell values render as React text children, so they're escaped — no HTML/CSV
// injection. Parsing uses papaparse (RFC-4180: quoted fields, escaped quotes,
// embedded newlines). First row is treated as the header.
export function TableView({ content, delimiter }: Props) {
  const { t } = useTranslation('common');

  const { header, body, truncated, totalRows } = useMemo(() => {
    const result = Papa.parse<string[]>(content, {
      delimiter: delimiter || undefined,
      skipEmptyLines: 'greedy',
    });
    const all = (result.data as string[][]).filter((r) => !(r.length === 1 && r[0] === ''));
    const [head = [], ...rest] = all;
    return { header: head, body: rest.slice(0, MAX_ROWS), truncated: rest.length > MAX_ROWS, totalRows: rest.length };
  }, [content, delimiter]);

  if (header.length === 0) {
    return <div className="cw-preview-doc cw-table-empty">{t('preview.table_empty')}</div>;
  }

  return (
    <div className="cw-preview-table-wrap">
      <table className="cw-preview-table">
        <thead>
          <tr>{header.map((cell, i) => <th key={i} scope="col">{cell}</th>)}</tr>
        </thead>
        <tbody>
          {body.map((row, r) => (
            <tr key={r}>{row.map((cell, c) => <td key={c}>{cell}</td>)}</tr>
          ))}
        </tbody>
      </table>
      {truncated && (
        <div className="cw-table-note">{t('preview.table_truncated', { n: MAX_ROWS, total: totalRows })}</div>
      )}
    </div>
  );
}
