import type { JSX } from 'react';

export function SourceIcon({ sourceId, size = 18 }: { sourceId: string; size?: number }): JSX.Element {
  switch (sourceId) {
    case 'local':
      // Indigo folder icon
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path
            d="M2 5.5C2 4.67 2.67 4 3.5 4H7.5L9 5.5H14.5C15.33 5.5 16 6.17 16 7V13.5C16 14.33 15.33 15 14.5 15H3.5C2.67 15 2 14.33 2 13.5V5.5Z"
            fill="#4F46E5"
          />
          <path
            d="M2 7.5H16V13.5C16 14.33 15.33 15 14.5 15H3.5C2.67 15 2 14.33 2 13.5V7.5Z"
            fill="#6366F1"
          />
        </svg>
      );

    case 'gdrive':
      // Google Drive multi-color triangle
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          {/* Blue left triangle */}
          <path d="M2 14.5L6 7.5L10 14.5H2Z" fill="#4285F4" />
          {/* Yellow top triangle */}
          <path d="M6 7.5L10 7.5L14 14.5L10 14.5L6 7.5Z" fill="#FBBC04" />
          {/* Green right side */}
          <path d="M6.5 3.5L10.5 3.5L16 14.5L12 14.5L6.5 3.5Z" fill="#34A853" />
        </svg>
      );

    case 's3':
      // Amazon S3 orange bucket icon
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <ellipse cx="9" cy="5" rx="6" ry="2.5" fill="#E25444" />
          <rect x="3" y="5" width="12" height="8" fill="#E25444" opacity="0.8" />
          <ellipse cx="9" cy="13" rx="6" ry="2.5" fill="#C84133" />
          <ellipse cx="9" cy="5" rx="6" ry="2.5" fill="#FF7A6B" />
        </svg>
      );

    case 'dropbox':
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M5 2.5L9 5L5 7.5L1 5L5 2.5Z" fill="#0061FF" />
          <path d="M13 2.5L17 5L13 7.5L9 5L13 2.5Z" fill="#0061FF" />
          <path d="M5 8.5L9 11L5 13.5L1 11L5 8.5Z" fill="#0061FF" />
          <path d="M13 8.5L17 11L13 13.5L9 11L13 8.5Z" fill="#0061FF" />
          <path d="M9 11.8L13 14.2L9 16.5L5 14.2L9 11.8Z" fill="#4C8DFF" />
        </svg>
      );

    case 'figma':
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="6.5" cy="4.5" r="2.5" fill="#F24E1E" />
          <circle cx="11.5" cy="4.5" r="2.5" fill="#FF7262" />
          <circle cx="6.5" cy="9" r="2.5" fill="#A259FF" />
          <circle cx="11.5" cy="9" r="2.5" fill="#1ABCFE" />
          <circle cx="6.5" cy="13.5" r="2.5" fill="#0ACF83" />
        </svg>
      );

    case 'confluence':
      // Atlassian Confluence blue wave mark
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path
            d="M2.5 13.5C4.5 11 6.5 9.5 9 9.5C11.5 9.5 13.5 11 15.5 13.5"
            stroke="#1868DB"
            strokeWidth="2.5"
            strokeLinecap="round"
          />
          <path
            d="M2.5 8.5C4.5 6 6.5 4.5 9 4.5C11.5 4.5 13.5 6 15.5 8.5"
            stroke="#1868DB"
            strokeWidth="2.5"
            strokeLinecap="round"
          />
        </svg>
      );

    case 'notion':
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <rect x="2.5" y="2.5" width="13" height="13" rx="2" fill="white" stroke="#111827" strokeWidth="1.4" />
          <path d="M5.4 6.1H7.2L10.7 11.2V6.1H12.5" stroke="#111827" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
          <path d="M5.6 11.9H7.2V6.4" stroke="#111827" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      );

    case 'jira':
      // Jira blue diamond/compass icon
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M9 2L16 9L9 16L2 9L9 2Z" fill="#0052CC" />
          <path d="M9 5.5L12.5 9L9 12.5L5.5 9L9 5.5Z" fill="white" />
          <circle cx="9" cy="9" r="1.5" fill="#0052CC" />
        </svg>
      );

    case 'github':
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="9" cy="9" r="7" fill="#24292F" />
          <path
            d="M7 14C7.3 13.5 7.3 12.9 7.2 12.4C5.5 12.1 4.2 11.2 4.2 8.9C4.2 8.1 4.5 7.4 5 6.8C4.9 6.4 4.7 5.7 5.1 4.8C5.1 4.8 5.8 4.6 7.2 5.5C7.8 5.3 8.4 5.2 9 5.2C9.6 5.2 10.2 5.3 10.8 5.5C12.2 4.6 12.9 4.8 12.9 4.8C13.3 5.7 13.1 6.4 13 6.8C13.5 7.4 13.8 8.1 13.8 8.9C13.8 11.2 12.5 12.1 10.8 12.4C10.7 12.9 10.7 13.5 11 14"
            stroke="white"
            strokeWidth="1.2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      );

    case 'linear':
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <rect x="2" y="2" width="14" height="14" rx="4" fill="#5E6AD2" />
          <path d="M5 11.7L11.7 5" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
          <path d="M5 14L14 5" stroke="white" strokeWidth="1.5" strokeLinecap="round" opacity="0.72" />
          <path d="M5 9.4L9.4 5" stroke="white" strokeWidth="1.5" strokeLinecap="round" opacity="0.72" />
        </svg>
      );

    case 'gmail':
      // Google Gmail red envelope icon
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <rect x="2" y="4" width="14" height="10" rx="1.5" fill="white" stroke="#EA4335" strokeWidth="1.5" />
          <path d="M2 5.5L9 10.5L16 5.5" stroke="#EA4335" strokeWidth="1.5" strokeLinecap="round" />
          <path d="M2 5.5L5.5 9" stroke="#EA4335" strokeWidth="1.5" strokeLinecap="round" />
          <path d="M16 5.5L12.5 9" stroke="#EA4335" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      );

    case 'slack':
      // Slack purple hashtag/bolt icon
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          {/* Slack multi-color pinwheel mark (simplified) */}
          <rect x="3" y="3" width="5" height="5" rx="1.5" fill="#611F69" />
          <rect x="10" y="3" width="5" height="5" rx="1.5" fill="#1D9BD1" />
          <rect x="3" y="10" width="5" height="5" rx="1.5" fill="#ECB22E" />
          <rect x="10" y="10" width="5" height="5" rx="1.5" fill="#2BAC76" />
        </svg>
      );

    case 'knowledge':
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <rect x="2.5" y="2.5" width="13" height="13" rx="2.5" fill="#F8FAFC" stroke="#475569" strokeWidth="1.2" />
          <path d="M5.4 6.2H10.8" stroke="#475569" strokeWidth="1.2" strokeLinecap="round" />
          <path d="M5.4 9H8.6" stroke="#475569" strokeWidth="1.2" strokeLinecap="round" />
          <path d="M5.4 11.8H8.1" stroke="#475569" strokeWidth="1.2" strokeLinecap="round" />
          <path d="M10.2 11.2L11.4 12.4L14 9.5" stroke="#2563EB" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );

    default:
      // Generic folder/file fallback
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <rect x="2" y="4" width="14" height="11" rx="1.5" fill="#94A3B8" />
          <path d="M2 7H16" stroke="white" strokeWidth="1.5" />
        </svg>
      );
  }
}
