// Colored document-badge icon keyed by file extension.
// Renders a classic folded-corner document shape with the extension label
// (or a well-known abbreviation) on a brand-colored background — no external deps.

interface BadgeInfo { label: string; bg: string; fg: string; }

function badgeForExt(ext: string): BadgeInfo {
  switch (ext.toLowerCase()) {
    // ── Documents ────────────────────────────────────────────────────
    case 'pdf':             return { label: 'PDF', bg: '#E83E2C', fg: '#fff' };
    case 'doc': case 'docx': return { label: 'W',  bg: '#2B579A', fg: '#fff' };
    case 'hwp': case 'hwpx': return { label: 'HWP', bg: '#1A5EAB', fg: '#fff' };
    case 'odt':             return { label: 'ODT', bg: '#4B9BD5', fg: '#fff' };
    case 'rtf':             return { label: 'RTF', bg: '#6B7280', fg: '#fff' };
    case 'txt':             return { label: 'TXT', bg: '#6B7280', fg: '#fff' };
    case 'md': case 'markdown': return { label: 'MD', bg: '#374151', fg: '#fff' };
    case 'epub':            return { label: 'EPUB', bg: '#059669', fg: '#fff' };
    case 'pages':           return { label: 'PAG', bg: '#FF6B35', fg: '#fff' };
    // ── Presentations ────────────────────────────────────────────────
    case 'ppt': case 'pptx': return { label: 'P',  bg: '#D04423', fg: '#fff' };
    case 'odp':             return { label: 'ODP', bg: '#D97706', fg: '#fff' };
    case 'key':             return { label: 'KEY', bg: '#8B5CF6', fg: '#fff' };
    // ── Spreadsheets ─────────────────────────────────────────────────
    case 'xls': case 'xlsx': return { label: 'X',  bg: '#217346', fg: '#fff' };
    case 'csv':             return { label: 'CSV', bg: '#16A34A', fg: '#fff' };
    case 'tsv':             return { label: 'TSV', bg: '#15803D', fg: '#fff' };
    case 'ods':             return { label: 'ODS', bg: '#16A34A', fg: '#fff' };
    case 'numbers':         return { label: 'NUM', bg: '#34C759', fg: '#fff' };
    // ── Images ───────────────────────────────────────────────────────
    case 'png':             return { label: 'PNG', bg: '#7C3AED', fg: '#fff' };
    case 'jpg': case 'jpeg': return { label: 'JPG', bg: '#6D28D9', fg: '#fff' };
    case 'gif':             return { label: 'GIF', bg: '#C026D3', fg: '#fff' };
    case 'svg':             return { label: 'SVG', bg: '#F59E0B', fg: '#fff' };
    case 'webp':            return { label: 'WBP', bg: '#7C3AED', fg: '#fff' };
    case 'heic': case 'heif': return { label: 'HIC', bg: '#5B21B6', fg: '#fff' };
    case 'avif':            return { label: 'AVF', bg: '#7C3AED', fg: '#fff' };
    case 'tiff': case 'tif': return { label: 'TIF', bg: '#6D28D9', fg: '#fff' };
    case 'ico':             return { label: 'ICO', bg: '#8B5CF6', fg: '#fff' };
    case 'bmp':             return { label: 'BMP', bg: '#6D28D9', fg: '#fff' };
    case 'raw': case 'cr2': case 'nef': case 'arw':
                            return { label: 'RAW', bg: '#4C1D95', fg: '#fff' };
    // ── Video ────────────────────────────────────────────────────────
    case 'mp4':             return { label: 'MP4', bg: '#DC2626', fg: '#fff' };
    case 'mov':             return { label: 'MOV', bg: '#B91C1C', fg: '#fff' };
    case 'avi':             return { label: 'AVI', bg: '#991B1B', fg: '#fff' };
    case 'mkv':             return { label: 'MKV', bg: '#7F1D1D', fg: '#fff' };
    case 'webm':            return { label: 'WBM', bg: '#EF4444', fg: '#fff' };
    case 'wmv':             return { label: 'WMV', bg: '#B91C1C', fg: '#fff' };
    case 'flv':             return { label: 'FLV', bg: '#991B1B', fg: '#fff' };
    case 'm4v':             return { label: 'M4V', bg: '#DC2626', fg: '#fff' };
    // ── Audio ────────────────────────────────────────────────────────
    case 'mp3':             return { label: 'MP3', bg: '#D97706', fg: '#fff' };
    case 'wav':             return { label: 'WAV', bg: '#B45309', fg: '#fff' };
    case 'flac':            return { label: 'FLC', bg: '#92400E', fg: '#fff' };
    case 'aac':             return { label: 'AAC', bg: '#D97706', fg: '#fff' };
    case 'ogg':             return { label: 'OGG', bg: '#B45309', fg: '#fff' };
    case 'm4a':             return { label: 'M4A', bg: '#D97706', fg: '#fff' };
    case 'opus':            return { label: 'OPS', bg: '#B45309', fg: '#fff' };
    case 'weba':            return { label: 'WBA', bg: '#92400E', fg: '#fff' };
    case 'mid': case 'midi': return { label: 'MID', bg: '#78350F', fg: '#fff' };
    case 'aiff':            return { label: 'AIF', bg: '#D97706', fg: '#fff' };
    // ── Archives ─────────────────────────────────────────────────────
    case 'zip':             return { label: 'ZIP', bg: '#92400E', fg: '#fff' };
    case 'rar':             return { label: 'RAR', bg: '#78350F', fg: '#fff' };
    case '7z':              return { label: '7Z',  bg: '#78350F', fg: '#fff' };
    case 'tar':             return { label: 'TAR', bg: '#92400E', fg: '#fff' };
    case 'gz': case 'tgz':  return { label: 'GZ',  bg: '#78350F', fg: '#fff' };
    case 'bz2':             return { label: 'BZ2', bg: '#92400E', fg: '#fff' };
    case 'xz':              return { label: 'XZ',  bg: '#78350F', fg: '#fff' };
    case 'apk':             return { label: 'APK', bg: '#3DDC84', fg: '#fff' };
    case 'ipa':             return { label: 'IPA', bg: '#555', fg: '#fff' };
    case 'dmg':             return { label: 'DMG', bg: '#555', fg: '#fff' };
    case 'iso':             return { label: 'ISO', bg: '#4B5563', fg: '#fff' };
    case 'deb': case 'rpm': return { label: 'PKG', bg: '#4B5563', fg: '#fff' };
    // ── Code ─────────────────────────────────────────────────────────
    case 'js': case 'mjs': case 'cjs':
                            return { label: 'JS',  bg: '#F7DF1E', fg: '#1a1a1a' };
    case 'ts':              return { label: 'TS',  bg: '#3178C6', fg: '#fff' };
    case 'tsx':             return { label: 'TSX', bg: '#3178C6', fg: '#fff' };
    case 'jsx':             return { label: 'JSX', bg: '#61DAFB', fg: '#1a1a1a' };
    case 'py':              return { label: 'PY',  bg: '#3776AB', fg: '#FFD43B' };
    case 'rs':              return { label: 'RS',  bg: '#CE422B', fg: '#fff' };
    case 'go':              return { label: 'GO',  bg: '#00ADD8', fg: '#fff' };
    case 'java':            return { label: 'JV',  bg: '#ED8B00', fg: '#fff' };
    case 'kt': case 'kts':  return { label: 'KT',  bg: '#7F52FF', fg: '#fff' };
    case 'swift':           return { label: 'SW',  bg: '#FA7343', fg: '#fff' };
    case 'cs':              return { label: 'C#',  bg: '#9B4F96', fg: '#fff' };
    case 'cpp': case 'cc': case 'cxx':
                            return { label: 'C++', bg: '#00589C', fg: '#fff' };
    case 'c': case 'h': case 'hpp':
                            return { label: 'C',   bg: '#555',    fg: '#fff' };
    case 'php':             return { label: 'PHP', bg: '#777BB4', fg: '#fff' };
    case 'rb':              return { label: 'RB',  bg: '#CC342D', fg: '#fff' };
    case 'html': case 'htm': return { label: 'HTM', bg: '#E34F26', fg: '#fff' };
    case 'css':             return { label: 'CSS', bg: '#1572B6', fg: '#fff' };
    case 'scss': case 'sass': return { label: 'CSS', bg: '#CC6699', fg: '#fff' };
    case 'vue':             return { label: 'VUE', bg: '#42B883', fg: '#fff' };
    case 'svelte':          return { label: 'SVL', bg: '#FF3E00', fg: '#fff' };
    case 'json': case 'jsonc': return { label: 'JSN', bg: '#2d2d2d', fg: '#F7DF1E' };
    case 'yaml': case 'yml': return { label: 'YML', bg: '#CB171E', fg: '#fff' };
    case 'toml':            return { label: 'TML', bg: '#9C4221', fg: '#fff' };
    case 'sql':             return { label: 'SQL', bg: '#00758F', fg: '#fff' };
    case 'sh': case 'bash': case 'zsh': case 'fish':
                            return { label: 'SH',  bg: '#4EAA25', fg: '#fff' };
    case 'ps1':             return { label: 'PS1', bg: '#012456', fg: '#fff' };
    case 'xml':             return { label: 'XML', bg: '#F16529', fg: '#fff' };
    case 'dart':            return { label: 'DRT', bg: '#0175C2', fg: '#fff' };
    case 'r':               return { label: 'R',   bg: '#276DC3', fg: '#fff' };
    case 'lua':             return { label: 'LUA', bg: '#2C2D72', fg: '#fff' };
    case 'tf': case 'tfvars': return { label: 'TF', bg: '#7B42BC', fg: '#fff' };
    case 'graphql': case 'gql': return { label: 'GQL', bg: '#E535AB', fg: '#fff' };
    case 'proto':           return { label: 'PRT', bg: '#4285F4', fg: '#fff' };
    case 'dockerfile':      return { label: 'DKR', bg: '#2496ED', fg: '#fff' };

    default: {
      const upper = ext.toUpperCase().slice(0, 3);
      return { label: upper || '?', bg: '#9CA3AF', fg: '#fff' };
    }
  }
}

// Canonical viewBox: 20 wide × 24 tall (document proportions, fold = 6×6).
const VW = 20, VH = 24, FOLD = 6;
const DOC_PATH = `M2,0 L${VW - FOLD},0 L${VW},${FOLD} L${VW},${VH - 2} Q${VW},${VH} ${VW - 2},${VH} L2,${VH} Q0,${VH} 0,${VH - 2} L0,2 Q0,0 2,0 Z`;
const FOLD_PATH = `M${VW - FOLD},0 L${VW - FOLD},${FOLD} L${VW},${FOLD} Z`;

export function FileTypeIcon({ filename, size = 20 }: { filename: string; size?: number }) {
  const ext = filename.includes('.') ? (filename.split('.').pop() ?? '') : '';
  const { label, bg, fg } = badgeForExt(ext);
  const h = Math.round(size * (VH / VW));
  const n = label.length;
  const fontSize = n >= 4 ? 4.8 : n === 3 ? 5.5 : n === 2 ? 6.5 : 8;

  return (
    <svg
      viewBox={`0 0 ${VW} ${VH}`}
      width={size}
      height={h}
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
      focusable="false"
      style={{ display: 'inline-block', flexShrink: 0 }}
    >
      <path d={DOC_PATH} fill={bg} />
      <path d={FOLD_PATH} fill="rgba(255,255,255,0.22)" />
      <text
        x={VW / 2}
        y={VH * 0.72}
        textAnchor="middle"
        dominantBaseline="middle"
        fontSize={fontSize}
        fontFamily="-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif"
        fontWeight="800"
        fill={fg}
        letterSpacing="-0.03em"
      >
        {label}
      </text>
    </svg>
  );
}
