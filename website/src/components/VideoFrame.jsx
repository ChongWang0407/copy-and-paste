import { useRef, useState } from 'react';
import { VIDEO_SRC } from '../config';
import { PlayIcon } from './Icons';

// 视频框尺寸由 CSS 控制：宽度受视口高度约束，保证 16:9 画面完整落在一屏内。
// 播放开始时通知 Hero 收起宣传语、放大视频框；播放完毕再通知它恢复。
const VideoFrame = ({ onPlayingChange }) => {
  const frameRef = useRef(null);
  const videoRef = useRef(null);
  const [playing, setPlaying] = useState(false);
  const [nudge, setNudge] = useState(false);

  const handlePlay = () => {
    if (!VIDEO_SRC) {
      frameRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
      setNudge(true);
      window.setTimeout(() => setNudge(false), 2400);
      return;
    }
    setPlaying(true);
    videoRef.current?.play();
  };

  return (
    <div className="video-frame" ref={frameRef} id="video">
      <div className="video-inner">
        {VIDEO_SRC && (
          <video
            ref={videoRef}
            src={VIDEO_SRC}
            controls={playing}
            playsInline
            // 点播放才开始加载，避免入场动画时浏览器在后台下载、解码视频
            preload="none"
            onPlay={() => {
              setPlaying(true);
              onPlayingChange?.(true);
            }}
            onEnded={() => {
              setPlaying(false);
              onPlayingChange?.(false);
            }}
          />
        )}

        {/* 没播放和播放结束后，盖一层蓝底大 logo 封面，不露出视频画面 */}
        {!playing && (
          <div className="video-placeholder" aria-hidden="true">
            <div className="video-placeholder__glow" />
            <img src="./logo.png" alt="" className="video-placeholder__logo" />
          </div>
        )}

        {!playing && (
          <button type="button" className="video-cover" onClick={handlePlay} aria-label="播放宣传视频">
            <span className="video-pill">
              <span className="video-play">
                <PlayIcon />
              </span>
              <span className="video-caption">
                <strong>{nudge ? '视频马上就来' : '观看产品演示'}</strong>
                <small>{nudge ? 'The film is on its way' : VIDEO_SRC ? 'Watch the film' : '宣传视频即将上线 · Coming soon'}</small>
              </span>
            </span>
          </button>
        )}
      </div>
    </div>
  );
};

export default VideoFrame;
