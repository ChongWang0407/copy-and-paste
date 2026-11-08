// 品牌蓝渐变字（和 App 设置页底部字标一致）。
// 逐字动画的每个字都是独立的 inline-block，直接在父元素上 background-clip:text 会被子元素的 transform 打断，
// 所以给每个字单独铺同一条渐变，再按它在整组里的横向位置平移背景，拼起来就是一条连续渐变。
// 位置用 offsetLeft 链算，不受动画中 transform/scale 的影响。

const xOf = el => {
  let x = 0;
  for (let n = el; n; n = n.offsetParent) x += n.offsetLeft;
  return x;
};

export const paintGradient = group => {
  if (!group.length) return;
  // 先加类：.grad-char 带内边距（再用负外边距抵消），让渐变区域盖住伸出字框的笔画，量位置要在这之后
  group.forEach(c => c.classList.add('grad-char'));
  const xs = group.map(xOf);
  const x0 = Math.min(...xs);
  const x1 = Math.max(...group.map((c, i) => xs[i] + c.offsetWidth));
  group.forEach((c, i) => {
    c.style.setProperty('--gw', `${x1 - x0}px`);
    c.style.setProperty('--gx', `${x0 - xs[i]}px`);
  });
};

// 在已拆分的字里找出指定单词（如 'Copy'、'Paste'），各自铺一条渐变
export const paintWords = (chars, words) => {
  const text = chars.map(c => c.textContent).join('');
  words.forEach(word => {
    let from = text.indexOf(word);
    while (from !== -1) {
      paintGradient(chars.slice(from, from + word.length));
      from = text.indexOf(word, from + word.length);
    }
  });
};
