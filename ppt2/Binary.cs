using System;
using System.IO;
using System.Linq;
using System.Text;

namespace Zetris.PPT {
    public static class Binary {
        static readonly int Version = 1;

        static byte[] CreateHeader() => Encoding.ASCII.GetBytes("TITN").Concat(BitConverter.GetBytes(Version)).ToArray();

        static void DecodeHeader(BinaryReader reader) {
            if (!reader.ReadChars(4).SequenceEqual(new char[] { 'T', 'I', 'T', 'N' })) throw new InvalidDataException();
            if (reader.ReadInt32() > Version) throw new InvalidDataException();
        }

        public static MemoryStream EncodePreferences() {
            MemoryStream output = new MemoryStream();

            using (BinaryWriter writer = new BinaryWriter(output)) {
                writer.Write(CreateHeader());
                writer.Write(Preferences.Personality);
                writer.Write(Preferences.Strength);
                writer.Write(Preferences.Speed);
                writer.Write(Preferences.HoldAllowed);
                writer.Write(Preferences.Player);
                writer.Write(Preferences.AccurateSync);
            }

            return output;
        }

        public static void DecodePreferences(FileStream input) {
            using (BinaryReader reader = new BinaryReader(input)) {
                DecodeHeader(reader);
                Preferences.Personality = reader.ReadInt32();
                Preferences.Strength = reader.ReadInt32();
                Preferences.Speed = reader.ReadInt32();
                Preferences.HoldAllowed = reader.ReadBoolean();
                Preferences.Player = reader.ReadInt32();
                Preferences.AccurateSync = reader.ReadBoolean();
            }
        }
    }
}
