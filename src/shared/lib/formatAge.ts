const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** Age of an entry, the same in every language: `< 1 min`, `5 min`, `3 h`, `2 d`. */
export const formatAge = (timestamp: number, now = Date.now()): string => {
  if (!timestamp || Number.isNaN(timestamp)) return "";
  const ms = timestamp < 1e11 ? timestamp * 1000 : timestamp;
  const age = now - ms;
  if (age < MINUTE) return "< 1 min"; // also a clock that ran backwards
  if (age < HOUR) return `${Math.floor(age / MINUTE)} min`;
  if (age < DAY) return `${Math.floor(age / HOUR)} h`;
  return `${Math.floor(age / DAY)} d`;
};
