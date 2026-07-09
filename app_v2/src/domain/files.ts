// File-type classification and preview-kind resolution.
// fileTypeClass and fileTypeIcon share the same categorise() source so colour
// and icon never drift. resolvePreviewKind is intentionally separate taxonomy.

type FileCategory = 'pdf' | 'sheet' | 'image' | 'doc' | 'code' | 'archive' | 'video' | 'audio' | 'other';

function categorise(name: string): FileCategory {
  const lower = name.toLowerCase();
  if (lower.endsWith('.pdf')) return 'pdf';
  if (/\.(csv|tsv|xlsx|xls|ods|numbers)$/.test(lower)) return 'sheet';
  if (/\.(png|jpe?g|gif|webp|svg|bmp|heic|heif|avif|tiff?|ico|raw|cr2|nef|arw)$/.test(lower)) return 'image';
  if (/\.(md|txt|doc|docx|markdown|rtf|odt|hwp|hwpx|pages|pptx?|odp|key|epub)$/.test(lower)) return 'doc';
  if (/\.(js|mjs|cjs|ts|tsx|jsx|json|jsonc|html|htm|css|scss|sass|less|py|rb|rs|go|java|kt|kts|c|cc|cpp|h|hpp|cs|fs|fsx|swift|m|sh|bash|zsh|fish|ps1|bat|cmd|yml|yaml|toml|xml|sql|graphql|gql|lua|php|r|scala|clj|cljs|ex|exs|erl|hrl|hs|ml|mli|nim|zig|dart|vue|svelte|elm|tf|tfvars|bicep|proto|gradle|groovy|cmake|make|dockerfile|ini|env|cfg|conf)$/.test(lower)) return 'code';
  if (/\.(zip|tar|gz|tgz|bz2|xz|lz4|zst|rar|7z|cab|apk|ipa|deb|rpm|dmg|pkg|iso)$/.test(lower)) return 'archive';
  if (/\.(mp4|mov|avi|mkv|webm|m4v|wmv|flv|3gp|ts|mts|m2ts|vob|ogv|rmvb)$/.test(lower)) return 'video';
  if (/\.(mp3|wav|flac|ogg|m4a|aac|weba|opus|mid|midi|aiff|au|wma|amr|ape|dsf)$/.test(lower)) return 'audio';
  return 'other';
}

export function fileTypeClass(name: string): string {
  switch (categorise(name)) {
    case 'pdf':     return 'cw-file-pdf';
    case 'sheet':   return 'cw-file-sheet';
    case 'image':   return 'cw-file-image';
    case 'doc':     return 'cw-file-doc';
    case 'code':    return 'cw-file-code';
    case 'archive': return 'cw-file-archive';
    case 'video':   return 'cw-file-video';
    case 'audio':   return 'cw-file-audio';
    default:        return 'cw-file-file';
  }
}

export function fileTypeIcon(name: string): 'file' | 'file-text' | 'sheet' | 'image' | 'file-pdf' | 'file-code' | 'file-archive' | 'file-video' | 'file-audio' {
  switch (categorise(name)) {
    case 'pdf':     return 'file-pdf';
    case 'sheet':   return 'sheet';
    case 'image':   return 'image';
    case 'doc':     return 'file-text';
    case 'code':    return 'file-code';
    case 'archive': return 'file-archive';
    case 'video':   return 'file-video';
    case 'audio':   return 'file-audio';
    default:        return 'file';
  }
}

// Preview kind is intentionally distinct from category taxonomy:
// .html/.htm → html (live render), .csv/.tsv → table, .ini/.env/.cfg/.conf → text.
export type PreviewKind = 'pdf' | 'image' | 'html' | 'markdown' | 'code' | 'table' | 'text' | 'unsupported';

const CODE_EXT =
  /\.(js|mjs|cjs|ts|tsx|jsx|json|jsonc|css|scss|sass|less|py|rb|rs|go|java|kt|kts|c|cc|cpp|h|hpp|cs|fs|fsx|swift|m|sh|bash|zsh|fish|ps1|bat|cmd|yml|yaml|toml|xml|sql|graphql|gql|lua|php|r|scala|clj|cljs|ex|exs|erl|hrl|hs|ml|mli|nim|zig|dart|vue|svelte|elm|tf|tfvars|bicep|proto|gradle|groovy|cmake|make|dockerfile)$/;

export function resolvePreviewKind(filename: string): PreviewKind {
  const lower = filename.toLowerCase();
  if (lower.endsWith('.pdf')) return 'pdf';
  if (/\.(png|jpe?g|gif|webp|svg|bmp|avif)$/.test(lower)) return 'image';
  if (/\.html?$/.test(lower)) return 'html';
  if (/\.(md|markdown)$/.test(lower)) return 'markdown';
  if (CODE_EXT.test(lower)) return 'code';
  if (/\.(csv|tsv)$/.test(lower)) return 'table';
  if (/\.(txt|log|env|ini|cfg|conf)$/.test(lower)) return 'text';
  return 'unsupported';
}

export function previewCodeLang(filename: string): string {
  const ext = filename.toLowerCase().split('.').pop() ?? '';
  return ext === filename.toLowerCase() ? '' : ext;
}
