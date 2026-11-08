import { useRef } from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import { CapsuleIcon, GlassIcon, MotionIcon, ScanIcon, StackIcon, TranslateIcon } from './Icons';

gsap.registerPlugin(ScrollTrigger, useGSAP);

const FEATURES = [
  {
    icon: GlassIcon,
    title: '原生液态玻璃',
    en: 'Native Liquid Glass',
    body: 'macOS 26 用系统 Liquid Glass，更早的系统降级为毛玻璃。通透度可调，6 种主题色。'
  },
  {
    icon: MotionIcon,
    title: '丝滑动效',
    en: 'Silky-smooth motion',
    body: '伸缩动画交给系统 Core Animation，按屏幕刷新节奏驱动；长列表虚拟滚动，滑得再快也顺。'
  },
  {
    icon: StackIcon,
    title: '批量粘贴',
    en: 'Batch paste',
    body: '勾选多条一次性按顺序粘贴，可选换行、空格或标点连接。结束后剪贴板无损恢复。'
  },
  {
    icon: CapsuleIcon,
    title: '复制提示灵动胶囊',
    en: 'Copy capsule',
    body: '每次复制，屏幕顶部弹出小胶囊，显示复制了什么、来自哪个应用。有刘海的屏幕从刘海里长出来。'
  },
  {
    icon: TranslateIcon,
    title: '离线中英翻译',
    en: 'On-device translation',
    body: '文本卡片一键中英互译，使用 Apple Translation 框架，翻译在本机完成。'
  },
  {
    icon: ScanIcon,
    title: '图片文字识别',
    en: 'Search text inside images',
    body: 'Apple Vision 离线识别中英文，搜索能搜到图里的字，还能把图片直接粘贴为文字。'
  }
];

const Features = () => {
  const ref = useRef(null);

  useGSAP(
    () => {
      if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
      gsap.from('.feature-card', {
        y: 36,
        opacity: 0,
        duration: 0.9,
        ease: 'power3.out',
        stagger: 0.08,
        scrollTrigger: { trigger: ref.current, start: 'top 80%', once: true }
      });
      gsap.from('.features-rule', {
        scaleX: 0,
        opacity: 0,
        duration: 1.2,
        ease: 'expo.out',
        scrollTrigger: { trigger: ref.current, start: 'top 90%', once: true }
      });
    },
    { scope: ref }
  );

  return (
    <section className="features" id="features" ref={ref}>
      <hr className="features-rule" />
      <div className="feature-grid">
        {FEATURES.map(({ icon: Icon, title, en, body }) => (
          <article className="feature-card" key={title}>
            <span className="feature-icon">
              <Icon />
            </span>
            <h3>{title}</h3>
            <p className="feature-en">{en}</p>
            <p className="feature-body">{body}</p>
          </article>
        ))}
      </div>
    </section>
  );
};

export default Features;
