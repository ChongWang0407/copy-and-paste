const base = { width: 20, height: 20, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 1.7, strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true };

export const GitHubIcon = props => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" {...props}>
    <path d="M12 .5C5.65.5.5 5.65.5 12a11.5 11.5 0 0 0 7.86 10.92c.58.1.79-.25.79-.56v-2c-3.2.7-3.87-1.37-3.87-1.37-.53-1.33-1.28-1.69-1.28-1.69-1.05-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.56-.29-5.25-1.28-5.25-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.78 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.38-5.26 5.67.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5Z" />
  </svg>
);

export const AppleIcon = props => (
  <svg width="17" height="17" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" {...props}>
    <path d="M16.37 12.73c-.02-2.3 1.88-3.4 1.96-3.46-1.07-1.56-2.73-1.78-3.32-1.8-1.41-.14-2.76.83-3.47.83-.72 0-1.82-.81-3-.79-1.54.02-2.96.9-3.76 2.28-1.6 2.78-.41 6.9 1.15 9.16.76 1.1 1.67 2.34 2.86 2.3 1.15-.05 1.58-.74 2.97-.74 1.38 0 1.77.74 2.98.72 1.23-.02 2.01-1.12 2.76-2.23.87-1.28 1.23-2.52 1.25-2.58-.03-.01-2.39-.92-2.41-3.64ZM14.1 5.96c.63-.77 1.06-1.83.94-2.89-.91.04-2.01.61-2.67 1.37-.58.67-1.1 1.76-.96 2.8 1.02.08 2.06-.52 2.69-1.28Z" />
  </svg>
);

export const PlayIcon = props => (
  <svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" {...props}>
    <path d="M8 5.6v12.8a1 1 0 0 0 1.52.85l10.2-6.4a1 1 0 0 0 0-1.7L9.52 4.75A1 1 0 0 0 8 5.6Z" />
  </svg>
);

export const ArrowIcon = props => (
  <svg {...base} width="15" height="15" strokeWidth="2" {...props}>
    <path d="M7 17 17 7M8 7h9v9" />
  </svg>
);

export const GlassIcon = () => (
  <svg {...base}>
    <rect x="3.5" y="4.5" width="17" height="15" rx="4.5" />
    <path d="M7.5 9.5c1.4-1.8 3.4-2.4 5.5-2" opacity=".7" />
  </svg>
);

export const MotionIcon = () => (
  <svg {...base}>
    <path d="M3.5 16c3.5 0 4.5-8 8.5-8s5 8 8.5 8" />
    <path d="M3.5 20h17" opacity=".5" />
  </svg>
);

export const CapsuleIcon = () => (
  <svg {...base}>
    <rect x="5" y="3.5" width="14" height="6" rx="3" />
    <path d="M8.5 6.5h4" />
    <rect x="3.5" y="12.5" width="17" height="8" rx="3" opacity=".6" />
  </svg>
);

export const ScanIcon = () => (
  <svg {...base}>
    <path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2" />
    <path d="M8 10h8M8 14h5" />
  </svg>
);

export const StackIcon = () => (
  <svg {...base}>
    <rect x="4" y="3.5" width="12" height="9" rx="2.5" />
    <rect x="8" y="11.5" width="12" height="9" rx="2.5" />
    <path d="M13 15.5h3" />
  </svg>
);

export const TranslateIcon = () => (
  <svg {...base}>
    <path d="M4 5h8M8 3v2M10.5 5c-.6 3.6-2.9 6.4-6 7.8M6 8.4c1 1.7 2.6 3.1 4.4 3.9" />
    <path d="m12.5 20.5 3.5-8.5 3.5 8.5M13.7 17.6h4.6" />
  </svg>
);
