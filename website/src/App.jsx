import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { gsap } from 'gsap';
import Hero from './components/Hero';
import Features from './components/Features';
import MagneticButton from './components/MagneticButton';
import { AppleIcon, ArrowIcon, GitHubIcon } from './components/Icons';
import { DOWNLOAD_URL, GITHUB_URL } from './config';
import { paintGradient } from './gradientText';

const APP_NAME = 'Copy & Paste';
const INTRO_KEY = 'cp-intro-played';

// 开发调试：在控制台用 __gsap 快进动画
if (import.meta.env.DEV && typeof window !== 'undefined') window.__gsap = gsap;

const shouldPlayIntro = () => {
  if (typeof window === 'undefined') return false;
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return false;
  if (new URLSearchParams(window.location.search).has('intro')) return true;
  return sessionStorage.getItem(INTRO_KEY) !== '1';
};

const waitForFonts = () =>
  Promise.race([document.fonts?.ready ?? Promise.resolve(), new Promise(r => setTimeout(r, 700))]);

const App = () => {
  const [playIntro] = useState(shouldPlayIntro);
  // intro：开场动画中；hero：首屏内容开始入场
  const [stage, setStage] = useState(playIntro ? 'intro' : 'hero');
  const [scrolled, setScrolled] = useState(false);
  const navRef = useRef(null);
  const veilRef = useRef(null);

  // 标题栏的 Copy & Paste 用品牌蓝渐变字；字体加载后按真实字宽铺渐变
  useEffect(() => {
    let alive = true;
    const paint = () => alive && paintGradient([...navRef.current.querySelectorAll('.brand-char')]);
    waitForFonts().then(paint);
    document.fonts?.ready.then(paint);
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 12);
    onScroll();
    window.addEventListener('scroll', onScroll, { passive: true });
    return () => window.removeEventListener('scroll', onScroll);
  }, []);

  // 开场：logo 弹出 → 名称逐字滚入 → 外面长出玻璃胶囊并拉宽成标题栏 → 标题栏上移到顶部
  // 全部在真实的导航栏元素上做变换，所以最后一帧和静止状态像素对齐。
  useLayoutEffect(() => {
    if (!playIntro) return undefined;
    const nav = navRef.current;
    const q = gsap.utils.selector(nav);
    let tl;
    let cancelled = false;

    document.documentElement.classList.add('is-intro');
    gsap.set(nav, { autoAlpha: 0 });

    const finish = () => {
      document.documentElement.classList.remove('is-intro');
      sessionStorage.setItem(INTRO_KEY, '1');
      setStage('hero');
    };

    waitForFonts().then(() => {
      if (cancelled) return;
      const logo = q('.brand-logo')[0];
      const name = q('.brand-name')[0];
      const chars = q('.brand-char');
      const pill = q('.nav-pill')[0];
      const github = q('.nav-right, .nav-tag');

      const navBox = nav.getBoundingClientRect();
      const logoBox = logo.getBoundingClientRect();
      const nameBox = name.getBoundingClientRect();

      const LOGO_SCALE = 2.4;
      const NAME_SCALE = 1.9;
      const GAP = 20;

      const ncx = navBox.left + navBox.width / 2;
      const dy = window.innerHeight / 2 - (navBox.top + navBox.height / 2);
      const lcx = logoBox.left + logoBox.width / 2;
      const mcx = nameBox.left + nameBox.width / 2;
      const bigLogo = logoBox.width * LOGO_SCALE;
      const bigName = nameBox.width * NAME_SCALE;
      const groupW = bigLogo + GAP + bigName;
      const logoSoloX = ncx - lcx;
      const logoGroupX = ncx - groupW / 2 + bigLogo / 2 - lcx;
      const nameGroupX = ncx - groupW / 2 + bigLogo + GAP + bigName / 2 - mcx;

      // 标题栏尺寸下，logo+名称在导航栏正中时各自需要的偏移
      const brandW = nameBox.right - logoBox.left;
      const centerShift = ncx - (logoBox.left + brandW / 2);
      // 胶囊从“刚好包住居中的 logo+名称”开始拉宽
      const inset = Math.max(0, (navBox.width - brandW - 40) / 2);

      gsap.set(nav, { autoAlpha: 1, y: dy });
      gsap.set(pill, { opacity: 0, clipPath: `inset(0px ${inset}px round 999px)` });
      gsap.set(github, { opacity: 0, x: -10 });
      gsap.set(logo, { x: logoSoloX, scale: 0.5, rotate: -18, opacity: 0 });
      gsap.set(name, { x: nameGroupX, scale: NAME_SCALE });
      gsap.set(chars, { yPercent: 115 });
      gsap.set(veilRef.current, { autoAlpha: 1 });

      tl = gsap.timeline({ onComplete: finish });
      tl.to(logo, { scale: LOGO_SCALE, rotate: 0, opacity: 1, duration: 0.75, ease: 'back.out(1.7)' }, 0.15)
        .to(logo, { x: logoGroupX, duration: 0.7, ease: 'expo.inOut' }, 0.85)
        .to(chars, { yPercent: 0, duration: 0.75, ease: 'expo.out', stagger: 0.028 }, 1.05)
        // 收拢成标题栏里的尺寸（仍居中），外圈浮现一颗小玻璃胶囊
        .to([logo, name], { x: centerShift, scale: 1, duration: 0.7, ease: 'expo.inOut' }, 1.9)
        .to(pill, { opacity: 1, duration: 0.4, ease: 'power1.out' }, 2.25)
        // 胶囊左右拉宽成标题栏，品牌同步滑到左侧，GitHub 按钮淡入
        .to(pill, { clipPath: 'inset(0px 0px round 999px)', duration: 0.85, ease: 'expo.inOut' }, 2.5)
        .to([logo, name], { x: 0, duration: 0.85, ease: 'expo.inOut' }, 2.5)
        .to(github, { opacity: 1, x: 0, duration: 0.5, ease: 'power2.out' }, 2.95)
        .to('.bg-orbs', { opacity: 1, duration: 1.2, ease: 'power1.inOut' }, 2.4)
        // 标题栏上移到顶部，首屏内容随后入场
        .to(nav, { y: 0, duration: 0.9, ease: 'expo.inOut' }, 3.2)
        .to(veilRef.current, { autoAlpha: 0, duration: 0.6, ease: 'power1.out' }, 3.3)
        .add(() => setStage('hero'), 3.6);

      // 开发调试：?intro&introAt=1.2 把开场定格在第 1.2 秒，方便逐帧检查
      const at = import.meta.env.DEV && new URLSearchParams(window.location.search).get('introAt');
      if (at) tl.pause(parseFloat(at), true);
    });

    const skip = () => tl?.progress(1);
    const veil = veilRef.current;
    veil.addEventListener('click', skip);
    window.addEventListener('keydown', skip);

    return () => {
      cancelled = true;
      veil.removeEventListener('click', skip);
      window.removeEventListener('keydown', skip);
      tl?.kill();
      gsap.set([nav, ...q('.brand-logo, .brand-name, .brand-char, .nav-pill, .nav-right, .nav-tag')], { clearProps: 'all' });
      document.documentElement.classList.remove('is-intro');
    };
  }, [playIntro]);

  return (
    <>
      <div className={`bg-orbs ${playIntro ? '' : 'is-on'}`} aria-hidden="true">
        <span className="orb orb-a" />
        <span className="orb orb-b" />
        <span className="orb orb-c" />
      </div>

      {playIntro && <div className="intro-veil" ref={veilRef} aria-hidden="true" />}

      <header className={`nav ${scrolled ? 'is-scrolled' : ''}`} ref={navRef}>
        <span className="nav-pill" aria-hidden="true" />
        <div className="nav-left">
        <a className="brand" href="#top" aria-label={APP_NAME}>
          <img className="brand-logo" src="./logo.png" alt="" width="34" height="34" />
          <span className="brand-name" aria-hidden="true">
            {[...APP_NAME].map((ch, i) => (
              <span className="brand-char" key={i}>
                {ch === ' ' ? ' ' : ch}
              </span>
            ))}
          </span>
        </a>
          <span className="nav-tag">
            <i />
            macOS 剪贴板工具 · 仅 8 MB 免费开源
          </span>
        </div>
        <div className="nav-right">
          <MagneticButton strength={0.3} maxDistance={8}>
            <a className="btn-rainbow btn-nav" href={DOWNLOAD_URL} target="_blank" rel="noreferrer">
              <AppleIcon />
              <span>Download</span>
            </a>
          </MagneticButton>
          <MagneticButton strength={0.3} maxDistance={8}>
            <a className="btn-rainbow btn-nav" href={GITHUB_URL} target="_blank" rel="noreferrer">
              <GitHubIcon />
              <span>GitHub</span>
              <ArrowIcon className="btn-nav__arrow" />
            </a>
          </MagneticButton>
        </div>
      </header>

      <main>
        {stage === 'hero' && (
          <>
            <Hero />
            <Features />
            <footer className="footer">
              <div className="footer-brand">
                <img src="./logo.png" alt="" width="26" height="26" />
                <span>Copy &amp; Paste</span>
              </div>
              <span className="footer-by">By WangChong</span>
            </footer>
          </>
        )}
      </main>
    </>
  );
};

export default App;
