using System.Runtime.InteropServices;
using System.Text;

namespace Zetris.PPT {
    /// Titan's own exports in MisaMino.dll (our engine DLL, see src/ppt.rs).
    static class Titan {
        /// Names for levels 11-15 (src/bot.rs): Titan's strength with a playing style.
        public static readonly string[] Personalities = { "Titan", "Trickster", "Chainlord", "Renegade", "Perfectionist", "Virtuoso" };

        [DllImport("MisaMino.dll")]
        static extern void titan_set_level(int level);

        [DllImport("MisaMino.dll")]
        static extern void titan_plan(StringBuilder str, int len);

        /// Personality 0 = Titan at `strength` 1-10; others play at full strength.
        public static void Configure(int personality, int strength) => titan_set_level(personality == 0? strength : 10 + personality);

        public static string Plan() {
            StringBuilder sb = new StringBuilder(200);
            titan_plan(sb, sb.Capacity);
            return sb.ToString();
        }
    }
}
