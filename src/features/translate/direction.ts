export type TranslateLanguage = "zh-Hans" | "en";

export interface TranslateDirection {
  source: TranslateLanguage;
  target: TranslateLanguage;
}

const HAN = /\p{Script=Han}/gu;
const LATIN = /\p{Script=Latin}/gu;

/**
 * 中文 → 英文, otherwise 英文 → 中文. A Chinese character carries about as much
 * as a short English word, so a few of them among Latin letters (产品名、型号)
 * still make the text Chinese.
 */
export const detectDirection = (text: string): TranslateDirection => {
  const han = text.match(HAN)?.length ?? 0;
  const latin = text.match(LATIN)?.length ?? 0;
  return han > 0 && han * 4 >= latin ? { source: "zh-Hans", target: "en" } : { source: "en", target: "zh-Hans" };
};
