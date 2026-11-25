import { expect, test } from "vitest";
import { setBounded } from "./boundedCache";

test("the cache keeps the most recently set entries up to its limit", () => {
  const cache = new Map<string, number>();
  for (let index = 0; index < 5; index++) setBounded(cache, `k${index}`, index, 3);
  expect([...cache.keys()]).toEqual(["k2", "k3", "k4"]);
  // Setting again moves it to the newest end.
  setBounded(cache, "k2", 20, 3);
  setBounded(cache, "k5", 5, 3);
  expect([...cache.entries()]).toEqual([
    ["k4", 4],
    ["k2", 20],
    ["k5", 5],
  ]);
});
