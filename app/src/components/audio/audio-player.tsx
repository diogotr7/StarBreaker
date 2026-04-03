import { useCallback, useEffect, useRef, useState } from "react";
import { useAudioStore } from "../../stores/audio-store";

export function AudioPlayer() {
  const currentSound = useAudioStore((s) => s.currentSound);
  const isPlaying = useAudioStore((s) => s.isPlaying);
  const progress = useAudioStore((s) => s.progress);
  const duration = useAudioStore((s) => s.duration);
  const setProgress = useAudioStore((s) => s.setProgress);
  const setPlaybackEnded = useAudioStore((s) => s.setPlaybackEnded);

  const audioRef = useRef<HTMLAudioElement>(null);
  const [volume, setVolume] = useState(0.5);

  // Listen for play/stop events from the store (blob URL owned by store)
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    audio.volume = volume;

    const onPlay = (e: Event) => {
      const { url } = (e as CustomEvent).detail;
      audio.src = url;
      audio.play();
    };

    const onStop = () => {
      audio.pause();
      audio.currentTime = 0;
    };

    window.addEventListener("audio-play", onPlay);
    window.addEventListener("audio-stop", onStop);
    return () => {
      window.removeEventListener("audio-play", onPlay);
      window.removeEventListener("audio-stop", onStop);
    };
  }, []);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    const onTimeUpdate = () => {
      setProgress(audio.currentTime, audio.duration || 0);
    };

    const onEnded = () => {
      setPlaybackEnded();
    };

    audio.addEventListener("timeupdate", onTimeUpdate);
    audio.addEventListener("ended", onEnded);
    return () => {
      audio.removeEventListener("timeupdate", onTimeUpdate);
      audio.removeEventListener("ended", onEnded);
    };
  }, [setProgress, setPlaybackEnded]);

  const onSeek = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const audio = audioRef.current;
      if (!audio || !duration) return;
      audio.currentTime = parseFloat(e.target.value) * duration;
    },
    [duration],
  );

  const togglePlay = useCallback(() => {
    const audio = audioRef.current;
    if (!audio) return;
    if (isPlaying) {
      audio.pause();
      useAudioStore.setState({ isPlaying: false });
    } else if (audio.src) {
      audio.play();
      useAudioStore.setState({ isPlaying: true });
    }
  }, [isPlaying]);

  const onVolume = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const v = parseFloat(e.target.value);
    setVolume(v);
    if (audioRef.current) audioRef.current.volume = v;
  }, []);

  const formatTime = (secs: number) => {
    if (!isFinite(secs) || secs < 0) return "0:00";
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
  };

  const seekValue = duration > 0 ? progress / duration : 0;

  return (
    <>
      {/* Always mounted so event listeners are registered */}
      <audio ref={audioRef} />

      {currentSound && (
        <div className="flex items-center gap-3 px-4 py-2 border-t border-border bg-bg-alt">
          <button
            type="button"
            onClick={togglePlay}
            className="w-8 h-8 flex items-center justify-center rounded-full bg-primary text-on-primary text-sm font-bold hover:opacity-90 transition-opacity"
          >
            {isPlaying ? "||" : "\u25B6"}
          </button>

          <input
            type="range"
            min={0}
            max={1}
            step={0.001}
            value={seekValue}
            onChange={onSeek}
            className="flex-1 h-1 accent-accent cursor-pointer"
          />

          <span className="text-xs text-text-dim font-mono min-w-[80px] text-right">
            {formatTime(progress)} / {formatTime(duration)}
          </span>

          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={volume}
            onChange={onVolume}
            className="w-20 h-1 accent-accent cursor-pointer"
            title={`Volume: ${Math.round(volume * 100)}%`}
          />

          <span className="text-xs text-text-faint font-mono">
            #{currentSound.media_id}
          </span>
        </div>
      )}
    </>
  );
}
