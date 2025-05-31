using System.Collections.Concurrent;
using System.Globalization;
using System.Numerics;
using System.Text;
using System.Xml.Linq;
using StarBreaker.Common;
using StarBreaker.CryXmlB;
using StarBreaker.DataCore;
using StarBreaker.P4k;

namespace StarBreaker.Sandbox;

public static class Crc32CSandbox
{
    public static void Run()
    {
        var uintsToTest = ReadKeys("keys.txt");

        var p4k = P4kDirectoryNode.FromP4k(P4kFile.FromFile(@"C:\Program Files\Roberts Space Industries\StarCitizen\4.0_PREVIEW\Data.p4k"));
        var dcbStream = p4k.OpenRead(@"Data\Game2.dcb");

        var dcb = new DataForge<string>(
            new DataCoreBinaryXml(
                new DataCoreDatabase(dcbStream)
            )
        );

        IEnumerable<string> haystack = [];

        haystack = new List<string>()
            //.Concat(EnumeratePaths(dcb.DataCore.Database.CachedStrings.Values, '/'))
            //.Concat(EnumeratePaths(dcb.DataCore.Database.CachedStrings2.Values, '/'))
            //.Concat(["head_eyedetail"])
            //.Concat(StreamLines("strings.txt"))
            .Concat(StreamLines("mats.txt"))
            .Concat(EnumeratePaths(StreamLines("mats.txt"), '/'))
            //.Concat(StreamLines("working.txt"))
            .Concat(EnumeratePaths(p4k.P4k.Entries.Select(x => x.Name), '\\'));

        //TODO: charactercustomizer_pu.socpak

        var result = BruteForce(uintsToTest, haystack);

        foreach (var (key, value) in result.OrderBy(x => x.Value.Count))
        {
            Console.WriteLine($"0x{key:X8} [{string.Join(", ", value)}]");
        }

        Console.WriteLine($"Number of found keys: {result.Values.Count(x => x.Count > 0)}");
        Console.WriteLine($"Number of missing keys: {result.Values.Count(x => x.Count == 0)}");
        return;
    }

    static IEnumerable<string> GetVariations(string str)
    {
        foreach (var s in str.Split('/'))
        {
            yield return s;
        }

        foreach (var s in str.Split('_'))
        {
            yield return s;
        }

        foreach (var s in str.Split('-'))
        {
            yield return s;
        }

        foreach (var s in str.Split(' '))
        {
            yield return s;
        }

        foreach (var s in GetNumbered(str))
        {
            yield return s;
        }

        yield return str.ToLower();
        yield return str.ToUpper();

        //return this last so it gets replaced in the dictionary if it matches
        yield return str;
    }

    static IEnumerable<string> GetNumbered(string str)
    {
        yield return str;
        for (var i = 0; i < 2; i++)
        {
            yield return $"{str}{i}";
        }
    }

    static uint[] ReadKeys(string file)
    {
        var lines = File.ReadAllLines(file);
        var keys = new List<uint>();

        foreach (var line in lines)
        {
            if (line.StartsWith("0x") &&
                uint.TryParse(line[2..], NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var key))
            {
                keys.Add(key);
            }
        }

        return keys.Distinct().Order().ToArray();
    }

    private static IEnumerable<string> StreamLines(string filePath)
    {
        using var reader = new StreamReader(File.OpenRead(filePath));

        while (reader.ReadLine() is { } line)
            yield return line;
    }

    private static IEnumerable<string> EnumeratePaths(IEnumerable<string> p4k, char separator)
    {
        foreach (var entry in p4k)
        {
            foreach (var part in entry.Split(separator))
            {
                yield return part;
            }
        }
    }

    /// <summary>
    ///     Brute force all possible combinations of strings and keys
    /// </summary>
    /// <param name="keys">crc32c results to test</param>
    /// <param name="strings">Original strings to modify and test against</param>
    private static ConcurrentDictionary<uint, HashSet<string>> BruteForce(uint[] keys, IEnumerable<string> strings)
    {
        var dict = new ConcurrentDictionary<uint, HashSet<string>>(keys.ToDictionary(key => key, _ => new HashSet<string>()));
        var tested = 0;

        Parallel.ForEach(strings, pr =>
        {
            var buffer = new byte[4096];

            foreach (var str in GetVariations(pr))
            {
                Interlocked.Increment(ref tested);

                var byteLength = Encoding.ASCII.GetBytes(str, buffer);
                var acc = 0xFFFFFFFFu;

                for (var i = 0; i < byteLength; i++)
                {
                    acc = BitOperations.Crc32C(acc, buffer[i]);
                    var crc = ~acc;
                    if (keys.Contains(crc))
                    {
                        dict[crc].Add(str[..(i + 1)]);
                    }
                }
            }
        });

        Console.WriteLine($"Number of tested strings: {tested}");

        return dict;
    }

    public static IEnumerable<string> GetMaterialStrings()
    {
        const string s = @"D:\StarCitizen\P4k\Data\Objects\Characters\Human";
        var strings = new HashSet<string>();
        foreach (var file in Directory.EnumerateFiles(s, "*.mtl", SearchOption.AllDirectories))
        {
            using var fs = File.OpenRead(file);
            if (!CryXml.TryOpen(fs, out var xml))
                continue;

            strings.UnionWith(xml.EnumerateAllStrings());
        }

        return strings;
    }
}