using System;
using System.IO;

using Zetris.PPT.Memory;

namespace Zetris.PPT {
    public static class Preferences {
        static readonly string UserPath = Path.Combine(
            Environment.GetEnvironmentVariable("USERPROFILE"),
            ".titan-ppt2"
        );

        static readonly string FilePath = Path.Combine(UserPath, "settings.bin");

        static bool Initialized = false;

        /// Index into Titan.Personalities.
        static int _personality = 0;
        public static int Personality {
            get => _personality;
            set {
                _personality = Math.Max(0, Math.Min(Titan.Personalities.Length - 1, value));
                PPTBot.Instance.UpdateConfig();
                Save();
            }
        }

        /// Titan's level 1-10 (only for the plain Titan personality).
        static int _strength = 10;
        public static int Strength {
            get => _strength;
            set {
                _strength = Math.Max(1, Math.Min(10, value));
                PPTBot.Instance.UpdateConfig();
                Save();
            }
        }

        static int _speed = 100;
        public static int Speed {
            get => _speed;
            set {
                _speed = Math.Max(10, Math.Min(100, value));
                Save();
            }
        }

        static bool _hold = true;
        public static bool HoldAllowed {
            get => _hold;
            set {
                _hold = value;
                PPTBot.Instance.UpdateConfig();
                Save();
            }
        }

        static int _player = 1;
        public static int Player {
            get => _player;
            set {
                _player = Math.Max(0, Math.Min(3, value));
                Save();
            }
        }

        static bool _accurate = true;
        public static bool AccurateSync {
            get => _accurate;
            set {
                _accurate = value;
                PPTBot.Instance.UpdatePriority();
                Save();
            }
        }

        public static void Save() {
            if (!Initialized) return;

            if (!Directory.Exists(UserPath)) Directory.CreateDirectory(UserPath);

            try {
                File.WriteAllBytes(FilePath, Binary.EncodePreferences().ToArray());
            } catch (IOException) {}
        }

        static Preferences() {
            GameHelper.UsePPT2();

            if (File.Exists(FilePath))
                using (FileStream file = File.Open(FilePath, FileMode.Open, FileAccess.Read))
                    try {
                        Binary.DecodePreferences(file);
                    } catch {}

            Initialized = true;
        }
    }
}
