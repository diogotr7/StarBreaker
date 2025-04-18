﻿using System.Text;
using StarBreaker.Chf;
using StarBreaker.Common;

namespace StarBreaker.Sandbox;

public static class ChfSandbox
{
    public static async Task Run()
    {
        var dnas = @"C:\Users\Diogo\Desktop\dnas.txt";
        var load = (await File.ReadAllLinesAsync(dnas)).Select(FixWeirdDnaString).ToArray();
        int i = 0;
        await CreateCharactersFromDnaStrings(load.Select(x => ($"female_{i++}.chf", x)));

        // await FixDnaStrings();
        //
        // var hugeData = Path.Combine(SandboxPaths.ResearchFolder, "dna", "huge_fixed.csv");
        // var huge_fixed = (await File.ReadAllLinesAsync(hugeData)).Select(x => x.Split(',')).Select(x => (x[1], x[0])).ToArray();
        // await CreateCharactersFromDnaStrings(huge_fixed);
    }

    private static Dictionary<FacePart, DnaPart[]> ParseDnaString(string dnaString)
    {
        var buffer = Convert.FromHexString(dnaString);
        var childReader = new SpanReader(buffer);
        var parts = new DnaPart[48];

        for (var i = 0; i < parts.Length; i++)
        {
            parts[i] = DnaPart.Read(ref childReader);
        }

        return parts.Select((part, idx) => (part, facePart: (FacePart)(idx % 12)))
            .GroupBy(x => x.facePart)
            .ToDictionary(x => x.Key, x => x.Select(y => y.part).ToArray());
    }

    private static async Task FigureOutHeadCount()
    {
        var dnaData = Path.Combine(SandboxPaths.ResearchFolder, "dna", "website.csv");
        var lines = (await File.ReadAllLinesAsync(dnaData)).Select(x => x.Split(',')).ToArray();
        var bytes = lines.Select(x => Convert.FromHexString(x[0])).Distinct().ToArray();

        var processed = bytes.Select(x =>
        {
            if (x.Length != 216)
                throw new Exception("length not 192");

            var count = x[0x16];
            var nonZeroParts = 0;
            var maxDataIndex = 0;
            var diffHeads = new HashSet<byte>();
            for (var i = 0; i < 48; i++)
            {
                var start = 0x18 + i * 4;
                var val1 = x[start];
                var val2 = x[start + 1];
                var headId = x[start + 2];

                if (x[start + 3] != 0)
                    throw new Exception("empty not 0");

                diffHeads.Add(headId);

                if (headId == 0 || (val1 == 0 && val2 == 0))
                    continue;

                nonZeroParts++;
                maxDataIndex = i;
            }

            return new
            {
                count,
                nonZeroParts,
                maxDataIndex,
                diffHeadCount = diffHeads.Count
            };
        }).ToArray();

        return;
    }

    private static void ExtractWebsiteDnas()
    {
        var websiteCharacters = Directory.GetFiles(SandboxPaths.WebsiteCharacters, "*.bin", SearchOption.AllDirectories);
        var characters2 = websiteCharacters.Select(x => (name: x, character: ChfData.FromBytes(File.ReadAllBytes(x)))).ToArray();
        var dnas = characters2.Select(p => $"{p.character.Dna.DnaString}, {Path.GetFileNameWithoutExtension(p.name)}").ToArray();

        File.WriteAllLines(Path.Combine(SandboxPaths.ResearchFolder, "dna", "website.csv"), dnas);
    }

    private static async Task FixDnaStrings()
    {
        var hugeData = Path.Combine(SandboxPaths.ResearchFolder, "dna", "huge.csv");
        var huge = (await File.ReadAllLinesAsync(hugeData)).Select(x => x.Split(',')).ToArray();
        var huge_fixed = huge
            .Where(x => x[0].Length == 384)
            .DistinctBy(x => x[0])
            .Select(x => (x[1], FixWeirdDnaString(x[0])))
            .ToArray();

        var huge_fixed_csv = huge_fixed.Select(x => $"{x.Item2},{x.Item1}");
        await File.WriteAllLinesAsync(Path.Combine(SandboxPaths.ResearchFolder, "dna", "huge_fixed.csv"), huge_fixed_csv);
    }

    public static async Task CreateCharactersFromDnaStrings(IEnumerable<(string name, string dna)> huge_fixed)
    {
        var dump = Path.Combine(SandboxPaths.ResearchFolder, "dump");
        Directory.CreateDirectory(dump);
        foreach (var (name, f1) in huge_fixed)
        {
            bool male;
            if (name.Contains("female", StringComparison.OrdinalIgnoreCase))
                male = false;
            else if (name.Contains("male", StringComparison.OrdinalIgnoreCase))
                male = true;
            else
                continue;

            var sanitized_name = name.Split('\\').Last();
            var path = Path.Combine(dump, $"{(male ? 'm' : 'f')}_{sanitized_name}.chf");

            await CreateCharacterFromDnaString(f1, path, male);
        }
    }

    private static async Task CreateCharacterFromDnaString(string dna, string name, bool male)
    {
        if (dna.Length != 384)
            throw new ArgumentException("Invalid length", nameof(dna));

        if (!name.EndsWith(".chf"))
            throw new ArgumentException("Not a chf file", nameof(name));

        const string bDirectory = @"C:\Development\StarCitizenChf\src\StarCitizenChf\bin\data\localCharacters";

        var default_c = male ? Path.Combine(bDirectory, "default_m", "default_m.chf") : Path.Combine(bDirectory, "default_f", "default_f.chf");
        var chf = ChfFile.FromChf(default_c);
        var dnaBytes = Convert.FromHexString(dna);

        //overwrite the dna
        const uint dnaStart = 0x30; //0x9493

        dnaBytes.CopyTo(chf.Data, dnaStart + 0x18);
        chf.Data[dnaStart + 0x16] = 0;
        //This^ is very strange. Setting the value to 0xff makes the character look like the default 0x00 one.
        //anything other than ff works?

        //Verify(chf);

        await chf.WriteToChfFileAsync(name);
    }

    // private static void Verify(ChfFile chf)
    // {
    //     var reader = new SpanReader(chf.Data);
    //     reader.Expect<uint>(2);
    //     reader.Expect<uint>(7);
    //
    //     var gender = BodyTypeChunk.Read(ref reader);
    //     var dna = DnaChunk.Read(ref reader);
    // }

    public static string FixWeirdDnaString(string dna)
    {
        if (dna.Length != 384)
            throw new ArgumentException("Invalid length", nameof(dna));

        var stringBuilder = new StringBuilder();

        //reverse endianness
        for (var i = 0; i < 48; i++)
        {
            var start = i * 8;
            var part = dna.Substring(start, 8);
            stringBuilder.Append(part[6..8]);
            stringBuilder.Append(part[4..6]);
            stringBuilder.Append(part[2..4]);
            stringBuilder.Append(part[0..2]);
        }

        return stringBuilder.ToString();
    }
}