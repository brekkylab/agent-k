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

    case 'jira':
      // Jira blue diamond/compass icon
      return (
        <svg width={size} height={size} viewBox="0 0 18 18" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M9 2L16 9L9 16L2 9L9 2Z" fill="#0052CC" />
          <path d="M9 5.5L12.5 9L9 12.5L5.5 9L9 5.5Z" fill="white" />
          <circle cx="9" cy="9" r="1.5" fill="#0052CC" />
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
