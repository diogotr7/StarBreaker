using System.Diagnostics;
using System.IO.Compression;
using System.Text.Json;
using StarBreaker.Common;
using StarBreaker.CryChunkFile;
using StarBreaker.P4k;

namespace StarBreaker.Sandbox;

public static class P4kSandbox
{
    private const string p4k = @"C:\Program Files\Roberts Space Industries\StarCitizen\PTU\Data.p4k";

    public static void Run()
    {
        TimeInit();
        //Verify();
        //ListByExtension();
        //CountEncrypted();
        //CountChunkTypes();
        // AllExtensions();
    }

    private static void TimeInit()
    {
        var timer = new TimeLogger();

        var p4kFile = P4kFile.FromFile(p4k);
        timer.LogReset("P4kFile.FromFile");
        var fs = P4kDirectoryNode.FromP4k(p4kFile);
        timer.LogReset("P4kDirectoryNode.FromP4k");
    }

    private static void AllExtensions()
    {
        var sw = Stopwatch.StartNew();
        var p4kFile = P4kFile.FromFile(p4k);

        var allNames = p4kFile.Entries.Select(x => x.Name).ToList();

        string[] removeList =
        [
            ".1",
            ".2",
            ".3",
            ".4",
            ".5",
            ".6",
            ".7",
            ".8",
            ".1a",
            ".2a",
            ".3a",
            ".4a",
            ".5a",
            ".6a",
            ".7a",
            ".8a",
            ".a"
        ];

        var trimmed = allNames.Select(x =>
        {
            //remove anything at the end that matches the removeList
            foreach (var remove in removeList)
            {
                if (x.EndsWith(remove, StringComparison.InvariantCultureIgnoreCase))
                {
                    x = x[..^remove.Length];
                }
            }
            
            return x;
        }).ToList();
        
        
        var exts = trimmed.Select(Path.GetExtension)
            .GroupBy(x => x)
            .Select(x => new { Ext = x.Key, Count = x.Count() })
            .OrderByDescending(x => x.Count)
            .ToList();
        
        File.WriteAllText(@"D:\StarCitizen\extensions.json", JsonSerializer.Serialize(exts));
    }

    private static void ListByExtension()
    {
        var sw = Stopwatch.StartNew();
        var p4kFile = P4kFile.FromFile(p4k);

        var ordered = p4kFile.Entries
            .Where(x => x.UncompressedSize > 0)
            .OrderBy(x => x.Offset);

        var failed = new List<P4kEntry>();

        var ivo = BitConverter.ToUInt32("#ivo"u8);
        var crch = BitConverter.ToUInt32("CrCh"u8);
        var exts = new List<string>();

        var cnt = 0;
        Parallel.ForEach(ordered, entry =>
        {
            try
            {
                using var bytes = p4kFile.OpenStream(entry);
                var binaryreader = new BinaryReader(bytes);
                var uint32 = binaryreader.ReadUInt32();
                if (uint32 == ivo)
                {
                    lock (exts)
                    {
                        exts.Add(entry.Name);
                    }
                }
            }
            catch (Exception e)
            {
                Console.WriteLine($"Failed to open {entry.Name}");
                failed.Add(entry);
            }

            Interlocked.Increment(ref cnt);

            if (cnt % 10000 == 0)
            {
                Console.WriteLine($"Processed {cnt} entries({(cnt / (double)p4kFile.Entries.Length) * 100}%)");
            }
        });

        foreach (var entry in failed)
        {
            Console.WriteLine($"Failed to open {entry.Name}");
        }

        File.WriteAllLines(@"C:\Scratch\StarCitizen\ivofiles.txt", exts);

        Console.WriteLine($"Took {sw.ElapsedMilliseconds}ms to load {p4kFile.Entries.Length} entries");
    }

    private static void CountEncrypted()
    {
        var sw = Stopwatch.StartNew();
        var p4kFile = P4kFile.FromFile(p4k);

        var uncompressed = p4kFile.Entries.Count(x => x.CompressionMethod == 0);
        var compressed = p4kFile.Entries.Count(x => x.CompressionMethod == 100 && x.IsCrypted == false);
        var encrypted = p4kFile.Entries.Count(x => x.IsCrypted == true);

        var encrypted2 = p4kFile.Entries.Where(x => x.IsCrypted).OrderBy(x => x.UncompressedSize).ToList();


        var total = uncompressed + compressed + encrypted;

        Console.WriteLine($"Uncompressed: {uncompressed}, {uncompressed / (double)total * 100}%");
        Console.WriteLine($"Compressed: {compressed}, {compressed / (double)total * 100}%");
        Console.WriteLine($"Encrypted: {encrypted}, {encrypted / (double)total * 100}%");

        Console.WriteLine($"Took {sw.ElapsedMilliseconds}ms to load {p4kFile.Entries.Length} entries");
    }

    private static void Verify()
    {
        var sw = Stopwatch.StartNew();
        var p4kFile = P4kFile.FromFile(p4k);

        var ordered = p4kFile.Entries.OrderBy(x => x.Offset);

        var failed = new List<P4kEntry>();

        var cnt = 0;
        Parallel.ForEach(ordered, entry =>
        {
            try
            {
                using var bytes = p4kFile.OpenStream(entry);
                var crc = Crc32c.FromStream(bytes);
                var matches = entry.Crc32 == crc;
                if (!matches)
                {
                    Console.WriteLine($"CRC32 mismatch for {entry.Name}");
                }
            }
            catch (Exception e)
            {
                Console.WriteLine($"Failed to open {entry.Name}");
                failed.Add(entry);
            }

            Interlocked.Increment(ref cnt);

            if (cnt % 10000 == 0)
            {
                Console.WriteLine($"Processed {cnt} entries({(cnt / (double)p4kFile.Entries.Length) * 100}%)");
            }
        });

        foreach (var entry in failed)
        {
            Console.WriteLine($"Failed to open {entry.Name}");
        }

        Console.WriteLine($"Took {sw.ElapsedMilliseconds}ms to load {p4kFile.Entries.Length} entries");
    }

    private static void CountChunkTypes()
    {
        var sw = Stopwatch.StartNew();
        var p4kFile = P4kFile.FromFile(p4k);

        var files = File.ReadAllLines(@"C:\Scratch\StarCitizen\ivofiles.txt");

        var chunktypes = new Dictionary<ChunkTypeIvo, int>();

        var cnt = 0;
        Parallel.ForEach(files, file =>
        {
            try
            {
                var entry = p4kFile.Entries.First(x => x.Name == file);
                var bytes = p4kFile.OpenStream(entry);
                if(!IvoFile.TryRead(bytes.ToArray(), out var fc))
                    throw new Exception("Failed to read ivo file");

                lock (chunktypes)
                {
                    foreach (var chunk in fc.Headers)
                    {
                        var type = chunk.ChunkType;
                        if (!chunktypes.TryAdd(type, 1))
                            chunktypes[type]++;
                    }
                }
                
                
                Interlocked.Increment(ref cnt);
                if (cnt % 10000 == 0)
                {
                    Console.WriteLine($"Processed {cnt} entries({(cnt / (double)files.Length) * 100}%)");
                }
            }
            catch (Exception e)
            {
                Console.WriteLine($"Failed to open {file}");
            }
        });
        
        foreach (var kvp in chunktypes)
        {
            Console.WriteLine($"{kvp.Key}: {kvp.Value}");
        }
    }
}