// 移植自 Aceternity UI 的 MagneticButton（TSX → JSX）。
// 磁吸跟随和弹簧参数与源码一致；源码 hover 时的蓝色虚线框去掉了，
// 改由 .magnetic 上的 is-active 类驱动黑色按钮的阴影和彩虹边（见 index.css）。
import { useRef, useState } from 'react';
import { motion, useReducedMotion } from 'motion/react';

export const MagneticButton = ({ children, strength = 0.8, maxDistance = 100, className = '' }) => {
  const ref = useRef(null);
  const [position, setPosition] = useState({ x: 0, y: 0 });
  const [hovered, setHovered] = useState(false);
  const reduce = useReducedMotion();

  const handleMouseMove = e => {
    if (!ref.current || reduce) return;

    const { width, height, left, top } = ref.current.getBoundingClientRect();
    const { clientX, clientY } = e;

    let x = (clientX - (left + width / 2)) * strength;
    let y = (clientY - (top + height / 2)) * strength;

    const distance = Math.hypot(x, y);
    if (distance > maxDistance) {
      const scale = maxDistance / distance;
      x *= scale;
      y *= scale;
    }

    setPosition({ x, y });
  };

  const handleMouseLeave = () => {
    setHovered(false);
    setPosition({ x: 0, y: 0 });
  };

  return (
    <div
      onMouseMove={handleMouseMove}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={handleMouseLeave}
      className={`magnetic ${hovered ? 'is-active' : ''} ${className}`.trim()}
    >
      <motion.div
        ref={ref}
        animate={{ x: position.x, y: position.y }}
        transition={{ type: 'spring', stiffness: 150, damping: 25, mass: 0.1 }}
      >
        {children}
      </motion.div>
    </div>
  );
};

export default MagneticButton;
