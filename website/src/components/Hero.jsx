import { useCallback, useEffect, useRef } from 'react';
import { gsap } from 'gsap';
import { useGSAP } from '@gsap/react';
import SplitText from './SplitText';
import VideoFrame from './VideoFrame';
import { paintWords } from '../gradientText';

gsap.registerPlugin(useGSAP);

// 宣传语逐字动画的参数严格按“宣传语文字效果.md”的用法示例
const SPLIT_PROPS = {
  effect: 'rise',
  splitType: 'chars',
  delay: 50,
  duration: 1.25,
  staggerFrom: 'start',
  trigger: 'view',
  hoverEffect: 'lift',
  loop: false,
  loopDelay: 1.5,
  mask: false,
  threshold: 0.1,
  rootMargin: '-100px',
  textAlign: 'center'
};

// 首屏 = 宣传语 + 视频框，正好占满一屏（.first-screen 高 100svh），
// 视频框在剩余高度里按 16:9 撑到最大，用户不用滚动就能看完整视频。
// 播放时：宣传语让出高度，视频框底边不动，向左、右、上三个方向长大，把两行字从上方顶出去（页面不滚动）。
// 播放完：视频框缩回，两行字从上方弹跳落回原位。
const EXPANDED_MAX = '1680px';

const Hero = () => {
  const ref = useRef(null);
  const headRef = useRef(null);
  const copyRef = useRef(null);
  const motionRef = useRef(null);

  const handlePlayingChange = useCallback(playing => {
    const root = ref.current;
    const copy = copyRef.current;
    const showcase = root.querySelector('.showcase');
    const lines = copy.children;
    const frame = root.querySelector('.video-frame');
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const k = reduce ? 0 : 1;
    // 两行字要移出页面顶端再多走一段：字块底边到页面顶 + 160px 余量，
    // 推出去后再设为隐藏，播放期间无论页面怎么滚、窗口怎么变，顶部都不会再露出来
    const outY = -(copy.getBoundingClientRect().top + window.scrollY + copy.scrollHeight + 160);
    motionRef.current?.kill();

    if (playing) {
      motionRef.current = gsap
        .timeline({ defaults: { duration: 0.9 * k, ease: 'power3.inOut' } })
        .to(copy, { height: 0 }, 0)
        .to(showcase, { marginTop: 0 }, 0)
        .to(root, { '--video-max': EXPANDED_MAX }, 0)
        // 白色边框淡出，变成无边框圆角视频
        .to(frame, { '--chrome': 0, duration: 0.6 * k, ease: 'power2.out' }, 0)
        // 字比视频框上沿走得快，始终在它前面被“顶”着出去
        .to(lines, { y: outY, ease: 'power3.in', duration: 0.7 * k, stagger: 0.06 * k }, 0)
        .set(lines, { visibility: 'hidden' });
      return;
    }

    // 读出样式表里的原始值，作为缩回的终点
    const prevMargin = showcase.style.marginTop;
    showcase.style.marginTop = '';
    const marginTop = getComputedStyle(showcase).marginTop;
    showcase.style.marginTop = prevMargin;
    const contentW = getComputedStyle(document.documentElement).getPropertyValue('--content-w').trim();

    motionRef.current = gsap
      .timeline({ defaults: { duration: 0.7 * k, ease: 'power3.inOut' } })
      .to(copy, { height: 'auto' }, 0)
      .to(showcase, { marginTop }, 0)
      .to(root, { '--video-max': contentW }, 0)
      // 白色边框随缩回渐渐出现
      .to(frame, { '--chrome': 1, duration: 0.8 * k, ease: 'power2.inOut' }, 0.1 * k)
      // 两行字从上方落下，落地后弹跳两下
      .set(lines, { visibility: 'visible' }, 0.2 * k)
      .fromTo(lines, { y: outY }, { y: 0, ease: 'bounce.out', duration: 1.1 * k, stagger: 0.12 * k }, 0.2 * k)
      .set(frame, { clearProps: '--chrome' })
      .set([copy, showcase, root, ...lines], { clearProps: 'height,marginTop,--video-max,transform,visibility' });
  }, []);

  // SplitText 拆字后，把 Copy、Paste 两个单词涂成品牌蓝渐变字
  useEffect(() => {
    const head = headRef.current;
    const paint = () => {
      const chars = [...head.querySelectorAll('.split-char')];
      if (chars.length) paintWords(chars, ['Copy', 'Paste']);
    };
    const mo = new MutationObserver(paint);
    mo.observe(head, { childList: true, subtree: true });
    window.addEventListener('resize', paint);
    paint();
    return () => {
      mo.disconnect();
      window.removeEventListener('resize', paint);
    };
  }, []);

  useGSAP(
    () => {
      if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
      const frame = ref.current.querySelector('.video-frame');
      // 入场时：位移交给 GPU 合成（force3D + will-change），毛玻璃暂时关掉，结束后再恢复
      frame.classList.add('is-entering');
      gsap.set(['.video-frame', '.horizon'], { willChange: 'transform, opacity', force3D: true });
      const tl = gsap.timeline({
        defaults: { ease: 'power3.out' },
        paused: true,
        onComplete: () => {
          frame.classList.remove('is-entering');
          gsap.set(['.video-frame', '.horizon'], { clearProps: 'willChange' });
        }
      });
      tl.from('.video-frame', { y: 70, opacity: 0, duration: 1.2, ease: 'expo.out', force3D: true }, 0.5)
        .from('.horizon', { opacity: 0, scaleX: 0.3, duration: 1.6, ease: 'expo.out', force3D: true }, 0.7);
      // 首屏刚挂载时浏览器还在排版、拆字、加载字体，等两帧再起跑，避免开头几帧被挤掉
      let raf = requestAnimationFrame(() => {
        raf = requestAnimationFrame(() => tl.play());
      });
      return () => {
        cancelAnimationFrame(raf);
        frame.classList.remove('is-entering');
      };
    },
    { scope: ref }
  );

  return (
    <div className="first-screen" ref={ref} id="top">
      <section className="hero">
        <div className="hero-copy" ref={copyRef}>
          <div className="slogan-main" ref={headRef}>
            <SplitText {...SPLIT_PROPS} tag="h1" className="slogan-en" text="Copy Anything and Paste Anywhere" />
          </div>
          <SplitText {...SPLIT_PROPS} tag="p" className="slogan-sub" text="Give Your Clipboard a Memory" />
        </div>
      </section>

      <section className="showcase">
        <div className="horizon" aria-hidden="true" />
        <VideoFrame onPlayingChange={handlePlayingChange} />
      </section>
    </div>
  );
};

export default Hero;
